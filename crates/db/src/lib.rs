//! crates/db — the only write path.
//! Readers: open additional read-only connections for concurrent UI queries
//! (WAL makes this safe). Writers: ONE connection behind a Mutex (db) or a
//! dedicated writer thread; keep transactions short (spec 5.2).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Db {
    pub conn: Connection,
}

pub fn apply_pragmas(c: &Connection) -> Result<()> {
    c.pragma_update(None, "busy_timeout", 5000i64)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.pragma_update(None, "temp_store", "MEMORY")?;
    c.pragma_update(None, "cache_size", -20000i64)?; // ~20MB page cache
    Ok(())
}

/// Ordered, idempotent migrations. Add a new tuple, never edit an applied one.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    ("0002_dedup_file_tags", include_str!("../migrations/0002_dedup_file_tags.sql")),
];

pub fn open(path: &Path) -> Result<Db> {
    let mut conn = Connection::open(path).with_context(|| format!("open {path:?}"))?;
    apply_pragmas(&conn)?;
    migrate(&mut conn)?;
    Ok(Db { conn })
}

fn migrate(c: &mut Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations(
           version TEXT PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);",
    )?;
    for (version, sql) in MIGRATIONS {
        let applied: Option<String> = c
            .query_row(
                "SELECT version FROM schema_migrations WHERE version = ?1",
                params![version],
                |r| r.get(0),
            )
            .optional()?;
        if applied.is_some() {
            continue;
        }
        let tx = c.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migration {version}"))?;
        tx.execute("INSERT INTO schema_migrations(version) VALUES (?1)", params![version])?;
        tx.commit()?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Provenance-aware tag writes (spec 4.2)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum AssignmentOutcome {
    Inserted,
    AlreadyActive,
    KeptDismissed, // a dismissed prediction must survive every future rescan
    Upgraded,
}

pub fn tag_id_by_name(c: &Connection, name: &str) -> Result<Option<i64>> {
    Ok(c.query_row("SELECT id FROM tags WHERE name = ?1", params![name], |r| r.get(0))
        .optional()?)
}

pub fn ensure_tag(c: &Connection, name: &str, kind: &str, origin: &str) -> Result<i64> {
    if let Some(id) = tag_id_by_name(c, name)? {
        return Ok(id);
    }
    c.execute(
        "INSERT INTO tags(name, kind, origin) VALUES (?1, ?2, ?3)",
        params![name, kind, origin],
    )?;
    Ok(c.last_insert_rowid())
}

/// Automatic sources (filename_rule | folder_rule | embedded | model | import).
/// The resurrection guard is the whole point: state='dismissed' wins forever
/// until a user manually re-adds.
pub fn record_automatic_tag(
    c: &Connection,
    file_id: i64,
    tag_id: i64,
    source: &str,
    confidence: Option<f32>,
    detail: Option<&str>,
) -> Result<AssignmentOutcome> {
    let existing: Option<(String, Option<f32>)> = c
        .query_row(
            "SELECT state, confidence FROM file_tags
             WHERE file_id=?1 AND tag_id=?2",
            params![file_id, tag_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match existing {
        Some((state, _)) if state == "dismissed" => Ok(AssignmentOutcome::KeptDismissed),
        Some((_, old_conf)) => {
            let new_conf = match (old_conf, confidence) {
                (Some(o), Some(n)) => Some(o.max(n)),
                (None, n) => n,
                (o, None) => o,
            };
            c.execute(
                "UPDATE file_tags SET confidence = ?3, source = ?4, source_detail = ?5,
                        updated_at = CURRENT_TIMESTAMP
                 WHERE file_id=?1 AND tag_id=?2",
                params![file_id, tag_id, new_conf, source, detail],
            )?;
            Ok(AssignmentOutcome::AlreadyActive)
        }
        None => {
            c.execute(
                "INSERT INTO file_tags(file_id, tag_id, source, confidence, source_detail)
                 VALUES (?1,?2,?3,?4,?5)",
                params![file_id, tag_id, source, confidence, detail],
            )?;
            Ok(AssignmentOutcome::Inserted)
        }
    }
}

pub fn add_manual_tag(c: &Connection, file_id: i64, tag_id: i64) -> Result<()> {
    c.execute(
        "INSERT INTO file_tags(file_id, tag_id, source, state)
         VALUES (?1,?2,'manual','confirmed')
         ON CONFLICT DO NOTHING",
        params![file_id, tag_id],
    )?;
    Ok(())
}

/// Manual confirmation upgrades ANY automatic assignment of that tag.
pub fn confirm_tag(c: &Connection, file_id: i64, tag_id: i64) -> Result<()> {
    c.execute(
        "UPDATE file_tags SET state='confirmed', updated_at=CURRENT_TIMESTAMP
         WHERE file_id=?1 AND tag_id=?2 AND source != 'manual'",
        params![file_id, tag_id],
    )?;
    Ok(())
}

/// Dismiss: soft-delete. The row stays so RescanRule 'never resurrect' holds.
pub fn dismiss_tag(c: &Connection, file_id: i64, tag_id: i64) -> Result<()> {
    c.execute(
        "UPDATE file_tags SET state='dismissed', updated_at=CURRENT_TIMESTAMP
         WHERE file_id=?1 AND tag_id=?2 AND source != 'manual'",
        params![file_id, tag_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scanner write helpers (called from the scanner's batched transaction)
// ---------------------------------------------------------------------------

/// Returns (file_id, changed). Change detection: size+mtime identity.
pub fn upsert_seen_file(
    c: &Connection,
    root_id: i64,
    rel: &str,
    name: &str,
    name_tokens: &str,
    ext: &str,
    size: i64,
    mtime: i64,
    run_id: i64,
) -> Result<(i64, bool)> {
    let prior: Option<(i64, i64, i64)> = c
        .query_row(
            "SELECT id, size, mtime FROM files WHERE root_id=?1 AND rel_path=?2",
            params![root_id, rel],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    match prior {
        Some((id, os, om)) if os == size && om == mtime => {
            c.execute(
                "UPDATE files SET last_seen_run=?1, missing_since=NULL WHERE id=?2",
                params![run_id, id],
            )?;
            Ok((id, false))
        }
        Some((id, _, _)) => {
            c.execute(
                "UPDATE files SET name=?2, name_tokens=?3, ext=?4, size=?5, mtime=?6,
                        last_seen_run=?7, missing_since=NULL,
                        duration_ms=NULL, sample_rate=NULL, content_id=NULL
                 WHERE id=?1",
                params![id, name, name_tokens, ext, size, mtime, run_id],
            )?;
            Ok((id, true)) // changed: invalidate staged metadata, re-queue
        }
        None => {
            c.execute(
                "INSERT INTO files(root_id, rel_path, name, name_tokens, ext, size, mtime, last_seen_run)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![root_id, rel, name, name_tokens, ext, size, mtime, run_id],
            )?;
            Ok((c.last_insert_rowid(), true))
        }
    }
}

/// End-of-run sweep: anything not seen this run is missing (retry-tolerant —
/// missing files restore themselves via missing_since=NULL on next sighting).
pub fn sweep_missing(c: &Connection, root_id: i64, run_id: i64) -> Result<usize> {
    let n = c.execute(
        "UPDATE files SET missing_since=CURRENT_TIMESTAMP
         WHERE root_id=?1 AND (last_seen_run IS NULL OR last_seen_run != ?2)
           AND missing_since IS NULL",
        params![root_id, run_id],
    )?;
    Ok(n)
}

pub fn get_tag(c: &Connection, id: i64) -> Result<Option<(String, String)>> {
    Ok(c.query_row(
        "SELECT name, kind FROM tags WHERE id=?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?)
}
