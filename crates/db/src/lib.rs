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

pub fn natural_cmp(s1: &str, s2: &str) -> std::cmp::Ordering {
    let mut it1 = s1.chars().peekable();
    let mut it2 = s2.chars().peekable();

    while let (Some(&c1), Some(&c2)) = (it1.peek(), it2.peek()) {
        if c1.is_ascii_digit() && c2.is_ascii_digit() {
            let mut digits1 = Vec::new();
            while let Some(&d) = it1.peek() {
                if d.is_ascii_digit() {
                    digits1.push(d);
                    it1.next();
                } else {
                    break;
                }
            }

            let mut digits2 = Vec::new();
            while let Some(&d) = it2.peek() {
                if d.is_ascii_digit() {
                    digits2.push(d);
                    it2.next();
                } else {
                    break;
                }
            }

            let nz1 = digits1.iter().skip_while(|&&d| d == '0').count();
            let nz2 = digits2.iter().skip_while(|&&d| d == '0').count();

            if nz1 != nz2 {
                return nz1.cmp(&nz2);
            }

            let sig1 = &digits1[digits1.len() - nz1..];
            let sig2 = &digits2[digits2.len() - nz2..];
            if sig1 != sig2 {
                return sig1.cmp(sig2);
            }

            if digits1.len() != digits2.len() {
                return digits1.len().cmp(&digits2.len());
            }
        } else {
            let l1 = c1.to_lowercase().collect::<Vec<_>>();
            let l2 = c2.to_lowercase().collect::<Vec<_>>();
            if l1 != l2 {
                return l1.cmp(&l2);
            }
            it1.next();
            it2.next();
        }
    }

    match (it1.next(), it2.next()) {
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        _ => std::cmp::Ordering::Equal,
    }
}

pub fn apply_pragmas(c: &Connection) -> Result<()> {
    c.pragma_update(None, "busy_timeout", 5000i64)?;
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.pragma_update(None, "temp_store", "MEMORY")?;
    c.pragma_update(None, "cache_size", -20000i64)?; // ~20MB page cache
    c.create_collation("NATURALNOCASE", natural_cmp)?;
    Ok(())
}

/// Ordered, idempotent migrations. Add a new tuple, never edit an applied one.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    ("0002_dedup_file_tags", include_str!("../migrations/0002_dedup_file_tags.sql")),
    ("0003_musical_keys_enharmonics", include_str!("../migrations/0003_musical_keys_enharmonics.sql")),
    ("0004_tag_kinds", include_str!("../migrations/0004_tag_kinds.sql")),
];

pub fn open(path: &Path) -> Result<Db> {
    let mut conn = Connection::open(path).with_context(|| format!("open {path:?}"))?;
    apply_pragmas(&conn)?;
    migrate(&mut conn)?;
    Ok(Db { conn })
}

fn migrate(c: &mut Connection) -> Result<()> {
    c.pragma_update(None, "foreign_keys", "OFF")?;
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
    c.pragma_update(None, "foreign_keys", "ON")?;
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
        if kind != "category" {
            let _ = c.execute(
                "UPDATE tags SET kind = ?1 WHERE id = ?2 AND kind = 'category'",
                params![kind, id],
            );
        }
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

pub fn find_musical_key(c: &Connection, key_or_camelot: &str) -> Result<Option<(i64, String, String)>> {
    let lower = key_or_camelot.to_ascii_lowercase();
    c.query_row(
        "SELECT id, label, camelot FROM musical_keys
         WHERE lower(label) = ?1 OR lower(camelot) = ?1 LIMIT 1",
        params![lower],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(Into::into)
}

pub fn update_file_musical_info(
    c: &Connection,
    file_id: i64,
    bpm: Option<f64>,
    key_id: Option<i64>,
    is_loop: Option<bool>,
) -> Result<()> {
    c.execute(
        "UPDATE files SET bpm = coalesce(?2, bpm),
                          key_id = coalesce(?3, key_id),
                          is_loop = coalesce(?4, is_loop)
         WHERE id = ?1",
        params![file_id, bpm, key_id, is_loop.map(|b| b as i64)],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_natural_sort() {
        let mut list = vec![
            "Perc 11.wav",
            "Perc 1.wav",
            "Perc 10.wav",
            "Perc 2.wav",
            "Perc 20.wav",
            "Perc 3.wav",
        ];
        list.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            list,
            vec![
                "Perc 1.wav",
                "Perc 2.wav",
                "Perc 3.wav",
                "Perc 10.wav",
                "Perc 11.wav",
                "Perc 20.wav",
            ]
        );
    }
}
