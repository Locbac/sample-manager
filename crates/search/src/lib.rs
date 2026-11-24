//! crates/search — list queries the UI pages through (spec 5.2)
//! Never loads the library: every query is LIMIT/OFFSET with a total
//! computed by a window count on the first page only.

use rusqlite::{types::Value, Connection, OptionalExtension};

#[derive(Debug, Default, Clone)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub root_id: Option<i64>,
    pub folder_prefix: Option<String>,
    pub exts: Vec<String>,
    pub dur_min_ms: Option<i64>,
    pub dur_max_ms: Option<i64>,
    pub bpm_min: Option<f64>,
    pub bpm_max: Option<f64>,
    pub key_id: Option<i64>,
    pub key_relative: bool,
    pub key_transposed: bool,
    pub is_loop: Option<bool>,
    pub favorites_only: bool,
    pub tags_include: Vec<i64>, // AND semantics (EXISTS per tag)
    pub tags_exclude: Vec<i64>,
    pub collection_id: Option<i64>,
    pub sort: Sort,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sort {
    NameAsc,
    PathAsc,
    DurationDesc,
    BpmAsc,
    SizeDesc,
    AddedDesc,
}

impl Default for Sort {
    fn default() -> Self {
        Sort::NameAsc
    }
}

#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: i64,
    pub rel_path: String,
    pub name: String,
    pub ext: String,
    pub duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub camelot: Option<String>,
    pub key_pc: Option<i32>,
    pub key_minor: Option<i32>,
    pub is_loop: Option<bool>,
    pub size: i64,
    pub favorite: bool,
    pub total: i64, // window count, valid on every row
    pub tags: Vec<String>,
    pub tags_display: String,
}

/// FTS5: user text -> quoted AND-joined prefix terms: 'open hat' => "open"* "hat"*
/// Quoting each term means FTS metacharacters in user input can't break the query.
pub fn fts_expr(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

pub fn search(c: &Connection, q: &SearchQuery) -> rusqlite::Result<Vec<FileRow>> {
    // whitelist ORDER BY — uses natural collation for intuitive numerical ordering
    let order = match q.sort {
        Sort::NameAsc => "f.name COLLATE NATURALNOCASE ASC, f.id",
        Sort::PathAsc => "f.rel_path COLLATE NATURALNOCASE ASC, f.id",
        Sort::DurationDesc => "f.duration_ms DESC NULLS LAST, f.id",
        Sort::BpmAsc => "f.bpm ASC NULLS LAST, f.id",
        Sort::SizeDesc => "f.size DESC, f.id",
        Sort::AddedDesc => "f.added_at DESC, f.id",
    };

    let mut where_sql: Vec<String> = vec!["f.missing_since IS NULL".into()];
    let mut params: Vec<Value> = Vec::new();
    let mut join_fts = false;

    if let Some(e) = q.q.as_ref().and_then(|s| fts_expr(s)) {
        join_fts = true;
        where_sql.push("files_fts MATCH ?".into());
        params.push(Value::from(e));
    }
    if let Some(id) = q.root_id {
        where_sql.push("f.root_id = ?".into());
        params.push(id.into());
    }
    if let Some(pref) = &q.folder_prefix {
        where_sql.push("f.rel_path LIKE ? || '%' ESCAPE '\\'".into());
        params.push(Value::from(pref.trim_end_matches('/').to_string()));
        // (defensive: ESCAPE clause reserved for when % / _ become user-controlled)
    }
    if !q.exts.is_empty() {
        let marks = q.exts.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        where_sql.push(format!("f.ext IN ({marks})"));
        for e in &q.exts {
            params.push(Value::from(e.clone()));
        }
    }
    if let Some(v) = q.dur_min_ms {
        where_sql.push("f.duration_ms >= ?".into());
        params.push(v.into());
    }
    if let Some(v) = q.dur_max_ms {
        where_sql.push("f.duration_ms <= ?".into());
        params.push(v.into());
    }
    if let Some(v) = q.bpm_min {
        where_sql.push("f.bpm >= ?".into());
        params.push(v.into());
    }
    if let Some(v) = q.bpm_max {
        where_sql.push("f.bpm <= ?".into());
        params.push(v.into());
    }
    let target_key = if let Some(kid) = q.key_id {
        c.query_row(
            "SELECT pc, minor, camelot FROM musical_keys WHERE id = ?",
            [kid],
            |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i32>(1)?, r.get::<_, String>(2)?)),
        )
        .optional()?
    } else {
        None
    };

    if let Some((target_pc, target_minor, _)) = target_key {
        if q.key_transposed {
            where_sql.push("f.key_id IS NOT NULL".into());
        } else if q.key_relative {
            let rel_pc = if target_minor == 1 { (target_pc + 3) % 12 } else { (target_pc + 9) % 12 };
            let rel_minor = 1 - target_minor;
            where_sql.push("((mk.pc = ? AND mk.minor = ?) OR (mk.pc = ? AND mk.minor = ?))".into());
            params.push(target_pc.into());
            params.push(target_minor.into());
            params.push(rel_pc.into());
            params.push(rel_minor.into());
        } else {
            where_sql.push("(mk.pc = ? AND mk.minor = ?)".into());
            params.push(target_pc.into());
            params.push(target_minor.into());
        }
    } else if q.key_id.is_some() {
        where_sql.push("1 = 0".into());
    }
    if let Some(v) = q.is_loop {
        where_sql.push("f.is_loop = ?".into());
        params.push((v as i64).into());
    }
    if q.favorites_only {
        where_sql.push("EXISTS(SELECT 1 FROM favorites fv WHERE fv.file_id = f.id)".into());
    }
    if let Some(cid) = q.collection_id {
        where_sql.push(
            "EXISTS(SELECT 1 FROM collection_items ci WHERE ci.collection_id = ? AND ci.file_id = f.id)".into(),
        );
        params.push(cid.into());
    }
    for t in &q.tags_include {
        where_sql.push(
            "EXISTS(SELECT 1 FROM file_tags ft WHERE ft.file_id = f.id AND ft.tag_id = ? AND ft.state != 'dismissed')".into(),
        );
        params.push((*t).into());
    }
    for t in &q.tags_exclude {
        where_sql.push(
            "NOT EXISTS(SELECT 1 FROM file_tags ft WHERE ft.file_id = f.id AND ft.tag_id = ? AND ft.state != 'dismissed')".into(),
        );
        params.push((*t).into());
    }

    let mut order_sql = order.to_string();
    if q.key_transposed {
        if let Some((target_pc, target_minor, _)) = target_key {
            let rel_pc = if target_minor == 1 { (target_pc + 3) % 12 } else { (target_pc + 9) % 12 };
            let target_tonic_expr = format!("(CASE WHEN mk.minor = {} THEN {} ELSE {} END)", target_minor, target_pc, rel_pc);
            let diff_expr = format!("(({target_tonic_expr} - mk.pc + 12) % 12)");
            let dist_expr = format!("min({diff_expr}, 12 - {diff_expr})");
            let modality_diff_expr = format!("(CASE WHEN mk.minor = {} THEN 0 ELSE 1 END)", target_minor);
            order_sql = format!("{dist_expr} ASC, {modality_diff_expr} ASC, {order}");
        }
    }

    let sql = format!(
        "SELECT f.id, f.rel_path, f.name, f.ext, f.duration_ms, f.bpm, f.size,
                EXISTS(SELECT 1 FROM favorites fv WHERE fv.file_id = f.id) AS fav,
                count(*) OVER () AS total,
                (
                    SELECT GROUP_CONCAT(DISTINCT t.name)
                    FROM file_tags ft
                    JOIN tags t ON t.id = ft.tag_id
                    WHERE ft.file_id = f.id AND ft.state != 'dismissed'
                ) AS tag_list,
                mk.label AS key_label,
                mk.camelot AS camelot_label,
                f.is_loop,
                mk.pc,
                mk.minor
         FROM files f
         LEFT JOIN musical_keys mk ON mk.id = f.key_id
         {fts}
         WHERE {w}
         ORDER BY {order_sql}
         LIMIT ? OFFSET ?",
        fts = if join_fts { "JOIN files_fts ON files_fts.rowid = f.id" } else { "" },
        w = where_sql.join(" AND "),
        order_sql = order_sql,
    );
    params.push((q.limit as i64).into());
    params.push((q.offset as i64).into());

    // keyset pagination TODO when OFFSET gets hot: ORDER BY (sort_col, id)
    // + WHERE (sort_col, id) > (?, ?) — measure first (spec 5.3).
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            let tags_raw: Option<String> = r.get(9)?;
            let mut tags: Vec<String> = Vec::new();
            if let Some(s) = tags_raw.as_deref() {
                let mut seen = std::collections::HashSet::new();
                for t in s.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()) {
                    if seen.insert(t.to_string()) {
                        tags.push(t.to_string());
                    }
                }
            }
            let tags_display = tags.join(", ");

            let key: Option<String> = r.get(10)?;
            let camelot: Option<String> = r.get(11)?;
            let is_loop_raw: Option<i64> = r.get(12)?;
            let is_loop = is_loop_raw.map(|v| v != 0);
            let key_pc: Option<i32> = r.get(13)?;
            let key_minor: Option<i32> = r.get(14)?;

            Ok(FileRow {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                name: r.get(2)?,
                ext: r.get(3)?,
                duration_ms: r.get(4)?,
                bpm: r.get(5)?,
                key,
                camelot,
                key_pc,
                key_minor,
                is_loop,
                size: r.get(6)?,
                favorite: r.get::<_, i64>(7)? == 1,
                total: r.get(8)?,
                tags,
                tags_display,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_natural_ordering() {
        let db = scout_db::Db {
            conn: rusqlite::Connection::open_in_memory().unwrap(),
        };
        scout_db::apply_pragmas(&db.conn).unwrap();
        db.conn
            .execute_batch(
                "CREATE TABLE roots (id INTEGER PRIMARY KEY, path TEXT UNIQUE, enabled INTEGER DEFAULT 1);
                 INSERT INTO roots (id, path) VALUES (1, '/test');
                 CREATE TABLE files (
                     id INTEGER PRIMARY KEY,
                     root_id INTEGER,
                     rel_path TEXT,
                     name TEXT,
                     ext TEXT,
                     size INTEGER,
                     mtime INTEGER,
                     duration_ms INTEGER,
                     bpm REAL,
                     key_id INTEGER,
                     is_loop INTEGER,
                     missing_since TEXT
                 );
                 CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE file_tags (file_id INTEGER, tag_id INTEGER, state TEXT);
                 CREATE TABLE favorites (file_id INTEGER PRIMARY KEY);
                 CREATE TABLE musical_keys (id INTEGER PRIMARY KEY, label TEXT, camelot TEXT, pc INTEGER, minor INTEGER);
                 INSERT INTO files (id, root_id, rel_path, name, ext, size, mtime) VALUES
                   (1, 1, 'Perc 11.wav', 'Perc 11.wav', 'wav', 100, 1),
                   (2, 1, 'Perc 1.wav', 'Perc 1.wav', 'wav', 100, 1),
                   (3, 1, 'Perc 2.wav', 'Perc 2.wav', 'wav', 100, 1),
                   (4, 1, 'Perc 10.wav', 'Perc 10.wav', 'wav', 100, 1),
                   (5, 1, 'Perc 20.wav', 'Perc 20.wav', 'wav', 100, 1);",
            )
            .unwrap();

        let q = SearchQuery {
            sort: Sort::NameAsc,
            limit: 50,
            ..Default::default()
        };
        let rows = search(&db.conn, &q).unwrap();
        let names: Vec<String> = rows.into_iter().map(|r| r.name).collect();
        assert_eq!(
            names,
            vec!["Perc 1.wav", "Perc 2.wav", "Perc 10.wav", "Perc 11.wav", "Perc 20.wav"]
        );
    }

    #[test]
    fn test_search_key_enharmonics_and_relative_and_transposed() {
        let db = scout_db::Db {
            conn: rusqlite::Connection::open_in_memory().unwrap(),
        };
        scout_db::apply_pragmas(&db.conn).unwrap();
        db.conn
            .execute_batch(
                "CREATE TABLE roots (id INTEGER PRIMARY KEY, path TEXT UNIQUE, enabled INTEGER DEFAULT 1);
                 INSERT INTO roots (id, path) VALUES (1, '/test');
                 CREATE TABLE files (
                     id INTEGER PRIMARY KEY,
                     root_id INTEGER,
                     rel_path TEXT,
                     name TEXT,
                     ext TEXT,
                     size INTEGER,
                     mtime INTEGER,
                     duration_ms INTEGER,
                     bpm REAL,
                     key_id INTEGER,
                     is_loop INTEGER,
                     missing_since TEXT
                 );
                 CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE file_tags (file_id INTEGER, tag_id INTEGER, state TEXT);
                 CREATE TABLE favorites (file_id INTEGER PRIMARY KEY);
                 CREATE TABLE musical_keys (id INTEGER PRIMARY KEY, label TEXT, camelot TEXT, pc INTEGER, minor INTEGER);
                 INSERT INTO musical_keys (id, label, camelot, pc, minor) VALUES
                   (1, 'Ebm', '2A', 3, 1),
                   (2, 'D#m', '2A', 3, 1),
                   (3, 'Am',  '8A', 9, 1),
                   (4, 'C',   '8B', 0, 0),
                   (5, 'Bm',  '10A', 11, 1),
                   (6, 'A#m', '3A', 10, 1),
                   (7, 'Gm',  '6A', 7, 1);
                 INSERT INTO files (id, root_id, rel_path, name, ext, size, mtime, key_id) VALUES
                   (1, 1, 'ebm_sample.wav', 'ebm_sample.wav', 'wav', 100, 1, 1),
                   (2, 1, 'dsharp_sample.wav', 'dsharp_sample.wav', 'wav', 100, 1, 2),
                   (3, 1, 'am_sample.wav', 'am_sample.wav', 'wav', 100, 1, 3),
                   (4, 1, 'c_sample.wav', 'c_sample.wav', 'wav', 100, 1, 4),
                   (5, 1, 'bm_sample.wav', 'bm_sample.wav', 'wav', 100, 1, 5),
                   (6, 1, 'asharp_sample.wav', 'asharp_sample.wav', 'wav', 100, 1, 6),
                   (7, 1, 'gm_sample.wav', 'gm_sample.wav', 'wav', 100, 1, 7);",
            )
            .unwrap();

        // 1. Enharmonic match: querying Ebm (id=1) matches both Ebm and D#m
        let q_enharmonic = SearchQuery {
            key_id: Some(1),
            limit: 50,
            ..Default::default()
        };
        let rows = search(&db.conn, &q_enharmonic).unwrap();
        let mut ids: Vec<i64> = rows.into_iter().map(|r| r.id).collect();
        ids.sort();
        assert_eq!(ids, vec![1, 2]);

        // 2. Relative match: querying Am (id=3) with key_relative = true matches Am (8A) and C (8B)
        let q_rel = SearchQuery {
            key_id: Some(3),
            key_relative: true,
            limit: 50,
            ..Default::default()
        };
        let rows = search(&db.conn, &q_rel).unwrap();
        let ids: Vec<i64> = rows.into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![3, 4]);

        // 3. Transposed match: querying Am (id=3) with key_transposed = true matches all keyed files
        // and sorts primarily by pitch distance |semitones| ASC:
        // Am (exact, dist 0) -> C (relative, dist 0) -> A#m (dist 1) -> Bm, Gm (dist 2) -> Ebm, D#m (dist 6)
        let q_trans = SearchQuery {
            key_id: Some(3),
            key_transposed: true,
            limit: 50,
            ..Default::default()
        };
        let rows = search(&db.conn, &q_trans).unwrap();
        let names: Vec<String> = rows.into_iter().map(|r| r.name).collect();
        assert_eq!(names[0], "am_sample.wav"); // exact (0st)
        assert_eq!(names[1], "c_sample.wav");  // relative (0st)
        assert_eq!(names[2], "asharp_sample.wav"); // -1st (dist 1)
        // Next are Bm (-2st, dist 2) and Gm (+2st, dist 2)
        assert!(names[3] == "bm_sample.wav" || names[3] == "gm_sample.wav");
        assert!(names[4] == "bm_sample.wav" || names[4] == "gm_sample.wav");

        // 4. Invalid key_id: fails closed (returns empty result, not all rows)
        let q_invalid = SearchQuery {
            key_id: Some(99999),
            limit: 50,
            ..Default::default()
        };
        let rows = search(&db.conn, &q_invalid).unwrap();
        assert_eq!(rows.len(), 0);
    }
}
