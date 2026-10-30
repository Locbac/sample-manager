-- scoutkit 0001_init.sql
-- Designed for THIS app. Not a clone of anything. FKs enforced via
-- PRAGMA foreign_keys=ON (set by the connection, not here).
-- Timestamps: ISO8601 text via DEFAULT CURRENT_TIMESTAMP unless noted.

CREATE TABLE roots (
  id                INTEGER PRIMARY KEY,
  path              TEXT NOT NULL UNIQUE,          -- absolute, normalized
  label             TEXT,
  enabled           INTEGER NOT NULL DEFAULT 1,
  file_count        INTEGER NOT NULL DEFAULT 0,
  last_scan_started_at  TEXT,
  last_scan_finished_at TEXT,
  last_error        TEXT,
  created_at        TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- One row PER PATH. Same audio at two paths = two rows, one content_id.
CREATE TABLE files (
  id            INTEGER PRIMARY KEY,
  root_id       INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
  rel_path      TEXT NOT NULL,                      -- relative to root
  name          TEXT NOT NULL,                      -- filename incl. ext
  name_tokens   TEXT NOT NULL DEFAULT '',           -- pre-tokenized by rule engine (FTS)
  ext           TEXT NOT NULL,
  size          INTEGER NOT NULL,
  mtime         INTEGER NOT NULL,                   -- unix seconds
  added_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_seen_run INTEGER,                            -- scan_runs.id that saw it
  missing_since TEXT,                               -- NULL = present
  -- cheap header metadata (stage 2, no full decode):
  duration_ms   INTEGER,
  sample_rate   INTEGER,
  channels      INTEGER,
  bit_depth     INTEGER,
  bpm           REAL,                               -- from metadata or lexical hint
  key_id        INTEGER REFERENCES musical_keys(id),
  is_loop       INTEGER,                            -- NULL unknown / 0 / 1 (wav smpl etc.)
  content_id    INTEGER REFERENCES contents(id),    -- NULL until background hash runs
  UNIQUE(root_id, rel_path)
);
CREATE INDEX ix_files_name     ON files(name);
CREATE INDEX ix_files_content  ON files(content_id);
CREATE INDEX ix_files_missing  ON files(root_id) WHERE missing_since IS NULL;
CREATE INDEX ix_files_bpm      ON files(bpm) WHERE bpm IS NOT NULL;
CREATE INDEX ix_files_duration ON files(duration_ms) WHERE duration_ms IS NOT NULL;

-- One row PER UNIQUE CONTENT. Heavy analysis hangs off this so duplicate
-- copies reuse analysis instead of recomputing.
CREATE TABLE contents (
  id               INTEGER PRIMARY KEY,
  hash             TEXT NOT NULL UNIQUE,            -- xxh3-128 hex (sha-256 ok too)
  size             INTEGER NOT NULL,
  duration_ms      INTEGER,                         -- ground truth after full decode
  sample_rate      INTEGER,
  channels         INTEGER,
  analysis_version INTEGER NOT NULL DEFAULT 0,      -- bump => reprocess
  analyzed_at      TEXT,
  first_seen_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE tags (
  id         INTEGER PRIMARY KEY,
  parent_id  INTEGER REFERENCES tags(id) ON DELETE SET NULL,
  name       TEXT NOT NULL,
  kind       TEXT NOT NULL DEFAULT 'category' CHECK (kind IN ('category','descriptor','mood','key','system')),
  color      TEXT,
  origin     TEXT NOT NULL DEFAULT 'user' CHECK (origin IN ('user','system')),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(parent_id, name)
);

-- The heart of provenance (spec 4.2). Every assignment knows WHERE it came
-- from, how sure it was, and its user state. Dismissed rows are KEPT so a
-- rescan never resurrects them (see db::record_automatic_tag).
CREATE TABLE file_tags (
  file_id      INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  tag_id       INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  source       TEXT NOT NULL CHECK (source IN
                 ('manual','filename_rule','folder_rule','embedded','model','import')),
  confidence   REAL,                                -- 0..1, NULL for manual
  source_detail TEXT,                               -- rule hit ('kick@bd'), model id+version, import table
  state        TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','confirmed','dismissed')),
  created_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
-- expression UNIQUE because source_detail may be NULL
CREATE UNIQUE INDEX uq_file_tags
  ON file_tags(file_id, tag_id, source, ifnull(source_detail, ''));
CREATE INDEX ix_file_tags_tag  ON file_tags(tag_id, state) ;
CREATE INDEX ix_file_tags_file ON file_tags(file_id, state);

CREATE TABLE favorites (
  file_id  INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE collections (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,
  kind         TEXT NOT NULL DEFAULT 'collection' CHECK (kind IN ('collection','playlist','smart')),
  smart_filter TEXT,                                -- JSON for kind='smart'
  created_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE collection_items (
  collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  file_id       INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  position      REAL NOT NULL DEFAULT 0,            -- REAL => cheap reorder; rebalance rarely
  added_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(collection_id, file_id)
);

-- Staged, bounded background work (spec 5.1). Single write path: only the
-- job worker mutates status; UI enqueues reader-visible items.
CREATE TABLE jobs (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('hash','header','peaks','classify','keybpm','embed')),
  file_id    INTEGER REFERENCES files(id) ON DELETE CASCADE,
  content_id INTEGER REFERENCES contents(id) ON DELETE CASCADE,
  priority   INTEGER NOT NULL DEFAULT 10,           -- 100 = user is looking at this folder
  status     TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','done','failed','cancelled')),
  payload    TEXT,                                  -- JSON
  attempts   INTEGER NOT NULL DEFAULT 0,
  run_after  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX ix_jobs_pick ON jobs(priority DESC, run_after, id) WHERE status = 'queued';

-- Analysis caches: separate from file rows so big blobs never ride along
-- in list queries (spec 5.2).
CREATE TABLE waveforms (
  content_id INTEGER PRIMARY KEY REFERENCES contents(id) ON DELETE CASCADE,
  version    INTEGER NOT NULL,
  peak_count INTEGER NOT NULL,
  peaks      BLOB NOT NULL,                          -- i16 min/max pairs
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE embeddings (
  content_id INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
  model      TEXT NOT NULL,
  dims       INTEGER NOT NULL,
  vec        BLOB NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (content_id, model)
);

-- ADSR importer traceability: old (table,id) -> new id. Lets a bad import be
-- unwound and lets users report exactly which rows came from where.
CREATE TABLE import_map (
  source       TEXT NOT NULL,                       -- 'adsr'
  source_table TEXT NOT NULL,
  source_id    INTEGER NOT NULL,
  target_table TEXT NOT NULL,
  target_id    INTEGER NOT NULL,
  imported_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(source, source_table, source_id, target_table)
);

CREATE TABLE scan_runs (
  id            INTEGER PRIMARY KEY,
  root_id       INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
  started_at    TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  finished_at   TEXT,
  files_seen    INTEGER NOT NULL DEFAULT 0,
  files_new     INTEGER NOT NULL DEFAULT 0,
  files_changed INTEGER NOT NULL DEFAULT 0,
  files_missing INTEGER NOT NULL DEFAULT 0,
  status        TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','done','cancelled','failed'))
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);

-- Musical keys: own table so ADSR enum notes can map in cleanly later.
CREATE TABLE musical_keys (
  id      INTEGER PRIMARY KEY,
  label   TEXT NOT NULL UNIQUE,   -- 'C#m'
  camelot TEXT NOT NULL,          -- '12A'
  pc      INTEGER NOT NULL,       -- pitch class 0-11 of tonic
  minor   INTEGER NOT NULL
);
INSERT INTO musical_keys (label, camelot, pc, minor) VALUES
 ('Cm','5A',0,1),('C','8B',0,0),('Dm','7A',2,1),('D','10B',2,0),
 ('Ebm','2A',3,1),('Eb','5B',3,0),('Em','9A',4,1),('E','12B',4,0),
 ('Fm','4A',5,1),('F','7B',5,0),('F#m','11A',6,1),('F#','2B',6,0),
 ('Gm','6A',7,1),('G','9B',7,0),('Abm','1A',8,1),('Ab','4B',8,0),
 ('Am','8A',9,1),('A','11B',9,0),('Bbm','3A',10,1),('Bb','6B',10,0),
 ('Bm','10A',11,1),('B','1B',11,0),('Dbm','12A',1,1),('Db','3B',1,0),
 ('Gbm','2A',6,1),('Gb','2B',6,0);

-- FTS5 external-content index over name + rel_path + scanner-provided
-- name_tokens (so 'kick_01' is searchable as 'kick' AND '01', and prefix
-- queries stay fast). Tag TEXT search goes through joins, keeping the FTS
-- delete-trigger trivially correct.
CREATE VIRTUAL TABLE files_fts USING fts5(
  name, rel_path, name_tokens,
  content='files', content_rowid='id'
);
CREATE TRIGGER files_fts_ins AFTER INSERT ON files BEGIN
  INSERT INTO files_fts(rowid, name, rel_path, name_tokens)
  VALUES (new.id, new.name, new.rel_path, new.name_tokens);
END;
CREATE TRIGGER files_fts_del AFTER DELETE ON files BEGIN
  INSERT INTO files_fts(files_fts, rowid, name, rel_path, name_tokens)
  VALUES ('delete', old.id, old.name, old.rel_path, old.name_tokens);
END;
CREATE TRIGGER files_fts_upd AFTER UPDATE ON files BEGIN
  INSERT INTO files_fts(files_fts, rowid, name, rel_path, name_tokens)
  VALUES ('delete', old.id, old.name, old.rel_path, old.name_tokens);
  INSERT INTO files_fts(rowid, name, rel_path, name_tokens)
  VALUES (new.id, new.name, new.rel_path, new.name_tokens);
END;
-- Ops note: after bulk maintenance you can always run
--   INSERT INTO files_fts(files_fts) VALUES('rebuild');
