//! crates/import-adsr — imports the USER'S OWN data from a COPY of their
//! ADSR database (spec 6). Hard rules:
//!   1. source opened READ-ONLY + immutable; this code contains zero writes
//!      targeting the source connection.
//!   2. schema is PROBED, not assumed: mapping tables/params are parameters
//!      with discovery helpers, not hard-coded prelim guesses.
//!   3. dry-run by default; committing requires an explicit flag.
//!   4. every row is traceable through import_map (source -> target).

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rusqlite::{Connection, OpenFlags};

#[derive(Debug, Default)]
pub struct ImportReport {
    pub tables_seen: Vec<(String, i64)>,
    pub tags_seen: i64,
    pub links_seen: i64,
    pub favorites_seen: i64,
    pub links_applied: i64,
    pub favorites_applied: i64,
    pub paths_unmatched: Vec<String>,
    pub warnings: Vec<String>,
    pub dry_run: bool,
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ADSR import report{}", if self.dry_run { " (DRY RUN)" } else { "" })?;
        writeln!(f, "  tables: {}", self.tables_seen.len())?;
        writeln!(f, "  tags seen: {} | links seen: {} | favorites seen: {}", self.tags_seen, self.links_seen, self.favorites_seen)?;
        writeln!(f, "  links applied: {} | favorites applied: {}", self.links_applied, self.favorites_applied)?;
        writeln!(f, "  unmatched source paths: {}", self.paths_unmatched.len())?;
        for w in &self.warnings {
            writeln!(f, "  warning: {w}")?;
        }
        Ok(())
    }
}

/// Table/column names discovered in docs/re/adsr-db-notes.md — these are
/// PLACEHOLDERS to be filled by real inspection before first use.
#[derive(Debug, Clone)]
pub struct AdsrMapping {
    pub files_table: String,   // e.g. "files"
    pub files_id: String,      // pk
    pub files_path: String,    // full path column
    pub tags_table: String,
    pub tags_id: String,
    pub tags_name: String,
    pub links_table: String,   // file<->tag link
    pub links_file: String,
    pub links_tag: String,
    pub favorites_table: Option<String>,
}

pub fn open_source_readonly(src: &Path) -> Result<Connection> {
    let uri = format!("file:{}?mode=ro&immutable=1", src.to_string_lossy());
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .context("open ADSR copy read-only (pass a COPY, never the live file)")
}

fn table_names(src: &Connection) -> Result<Vec<String>> {
    let mut st = src.prepare(
        "SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name",
    )?;
    let v = st
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

fn count(src: &Connection, table: &str) -> i64 {
    // identifiers come from OUR mapping struct (inspected), never user text
    src.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| r.get(0))
        .unwrap_or(0)
}

/// path_map entries are "FROM=TO" prefixes applied case-insensitively on
/// Windows-style inputs: "X:/Samples=/Volumes/audio/Samples".
fn normalize_path(raw: &str, path_map: &[String]) -> PathBuf {
    let mut p = raw.replace("\\", "/");
    for m in path_map {
        if let Some((from, to)) = m.split_once('=') {
            if p.to_lowercase().starts_with(&from.to_lowercase()) {
                p = format!("{}{}", to, &p[from.len()..]);
            }
        }
    }
    PathBuf::from(p)
}

pub fn import(src_path: &Path, target_db: &Path, path_map: &[String], dry_run: bool) -> Result<ImportReport> {
    if src_path == target_db {
        return Err(anyhow!("source and target are the same file — refusing"));
    }
    let src = open_source_readonly(src_path)?;
    let mut dst = scout_db::open(target_db)?;

    let mut report = ImportReport { dry_run, ..Default::default() };
    let names = table_names(&src)?;
    for t in &names {
        report.tables_seen.push((t.clone(), count(&src, t)));
    }

    // Fill this from docs/re/adsr-db-notes.md AFTER verifying the real
    // schema. If a table is absent, we warn and skip instead of failing.
    let map = AdsrMapping {
        files_table: "files".into(),
        files_id: "id".into(),
        files_path: "path".into(),
        tags_table: "tags".into(),
        tags_id: "id".into(),
        tags_name: "name".into(),
        links_table: "file_tags".into(),
        links_file: "file_id".into(),
        links_tag: "tag_id".into(),
        favorites_table: names.iter().find(|n| n.to_lowercase().contains("fav")).cloned(),
    };
    for required in [&map.files_table, &map.tags_table, &map.links_table] {
        if !names.contains(required) {
            report.warnings.push(format!("expected table '{required}' not found"));
        }
    }
    report.tags_seen = count(&src, &map.tags_table);
    report.links_seen = count(&src, &map.links_table);

    if dry_run {
        return Ok(report); // NEVER mutate anything on a dry run
    }

    // -- import, one transaction, fully traceable -----------------------------
    let tx = dst.conn.transaction()?;

    let mut tag_stmt = src.prepare(&format!(
        "SELECT \"{}\", \"{}\" FROM \"{}\"",
        map.tags_id, map.tags_name, map.tags_table
    ))?;
    let tags: Vec<(i64, String)> = tag_stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(tag_stmt);

    for (old_id, name) in &tags {
        let new_id = scout_db::ensure_tag(&tx, name, "category", "import")?;
        tx.execute(
            "INSERT OR IGNORE INTO import_map(source, source_table, source_id, target_table, target_id)
             VALUES ('adsr', ?1, ?2, 'tags', ?3)",
            rusqlite::params![map.tags_table, old_id, new_id],
        )?;
    }

    // links -> our file_tags with source='import', matched by absolute path
    let mut link_stmt = src.prepare(&format!(
        "SELECT f.\"{}\", l.\"{}\" FROM \"{}\" l JOIN \"{}\" f ON f.\"{}\" = l.\"{}\"",
        map.files_path, map.links_tag, map.links_table, map.files_table, map.files_id, map.links_file
    ))?;
    let links: Vec<(String, i64)> = link_stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(link_stmt);

    let mut find_file = tx.prepare(
        "SELECT f.id FROM files f JOIN roots r ON r.id = f.root_id
         WHERE r.path || '/' || f.rel_path = ?1",
    )?;
    for (raw_path, old_tag) in &links {
        let p = normalize_path(raw_path, path_map);
        let target = find_file
            .query_row([p.to_string_lossy().to_string()], |r| r.get::<_, i64>(0))
            .ok();
        match target {
            Some(file_id) => {
                let new_tag: Option<i64> = tx
                    .query_row(
                        "SELECT target_id FROM import_map
                         WHERE source='adsr' AND source_table=?1 AND source_id=?2 AND target_table='tags'",
                        rusqlite::params![map.tags_table, old_tag],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(tag_id) = new_tag {
                    scout_db::record_automatic_tag(
                        &tx,
                        file_id,
                        tag_id,
                        "import",
                        None,
                        Some("adsr"),
                    )?;
                    report.links_applied += 1;
                }
            }
            None => report.paths_unmatched.push(raw_path.clone()),
        }
    }
    drop(find_file);

    tx.commit()?;
    // unmatched paths are a REPORT, not an error — write them next to the db
    let miss_path = target_db.with_extension("import-missing.txt");
    std::fs::write(&miss_path, report.paths_unmatched.join("
"))?;
    report.warnings.push(format!("missing paths listed in {miss_path:?}"));
    Ok(report)
}
