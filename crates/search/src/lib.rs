//! crates/search — list queries the UI pages through (spec 5.2)
//! Never loads the library: every query is LIMIT/OFFSET with a total
//! computed by a window count on the first page only.

use rusqlite::{types::Value, Connection};

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

#[derive(Debug)]
pub struct FileRow {
    pub id: i64,
    pub rel_path: String,
    pub name: String,
    pub ext: String,
    pub duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    pub size: i64,
    pub favorite: bool,
    pub total: i64, // window count, valid on every row
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
    // whitelist ORDER BY — never interpolate raw user input here
    let order = match q.sort {
        Sort::NameAsc => "f.name COLLATE NOCASE ASC, f.id",
        Sort::PathAsc => "f.rel_path COLLATE NOCASE ASC, f.id",
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
    if let Some(v) = q.key_id {
        where_sql.push("f.key_id = ?".into());
        params.push(v.into());
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

    let sql = format!(
        "SELECT f.id, f.rel_path, f.name, f.ext, f.duration_ms, f.bpm, f.size,
                EXISTS(SELECT 1 FROM favorites fv WHERE fv.file_id = f.id) AS fav,
                count(*) OVER () AS total
         FROM files f {fts}
         WHERE {w}
         ORDER BY {order}
         LIMIT ? OFFSET ?",
        fts = if join_fts { "JOIN files_fts ON files_fts.rowid = f.id" } else { "" },
        w = where_sql.join(" AND "),
    );
    params.push((q.limit as i64).into());
    params.push((q.offset as i64).into());

    // keyset pagination TODO when OFFSET gets hot: ORDER BY (sort_col, id)
    // + WHERE (sort_col, id) > (?, ?) — measure first (spec 5.3).
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params), |r| {
            Ok(FileRow {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                name: r.get(2)?,
                ext: r.get(3)?,
                duration_ms: r.get(4)?,
                bpm: r.get(5)?,
                size: r.get(6)?,
                favorite: r.get::<_, i64>(7)? == 1,
                total: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
