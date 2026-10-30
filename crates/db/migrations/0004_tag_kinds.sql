-- Migration 0004: Expand tag kinds to support genre, instrument, character, etc.
PRAGMA foreign_keys=OFF;

CREATE TABLE tags_new (
  id         INTEGER PRIMARY KEY,
  parent_id  INTEGER REFERENCES tags_new(id) ON DELETE SET NULL,
  name       TEXT NOT NULL,
  kind       TEXT NOT NULL DEFAULT 'category' CHECK (kind IN ('category','genre','instrument','character','descriptor','mood','key','system')),
  color      TEXT,
  origin     TEXT NOT NULL DEFAULT 'user' CHECK (origin IN ('user','system')),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(parent_id, name)
);

INSERT INTO tags_new (id, parent_id, name, kind, color, origin, created_at)
  SELECT id, parent_id, name, kind, color, origin, created_at FROM tags;

DROP TABLE tags;
ALTER TABLE tags_new RENAME TO tags;

PRAGMA foreign_keys=ON;
