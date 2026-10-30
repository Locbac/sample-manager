// crates/app-tauri/src/commands.rs
// Tauri 2 command layer. Thin: it delegates to scout-db / scout-search /
// scout-scanner. The frontend NEVER constructs paths — ids in, paths out.
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub db: Mutex<scout_db::Db>, // single write connection; UI queries are short
    pub db_path: PathBuf,
    pub scan: Mutex<Option<scout_scanner::ScanHandle>>,
}

#[derive(Debug, Deserialize)]
pub struct SearchDto {
    pub q: Option<String>,
    pub root_id: Option<i64>,
    pub folder_prefix: Option<String>,
    pub tags_include: Vec<i64>,
    pub favorites_only: bool,
    pub sort: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Serialize)]
pub struct PageDto {
    pub rows: Vec<RowDto>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct RowDto {
    pub id: i64,
    pub rel_path: String,
    pub name: String,
    pub ext: String,
    pub duration_ms: Option<i64>,
    pub bpm: Option<f64>,
    pub favorite: bool,
}

#[tauri::command]
pub fn list_files(state: State<AppState>, dto: SearchDto) -> Result<PageDto, String> {
    let guard = state.db.lock().map_err(|e| e.to_string())?;
    let q = scout_search::SearchQuery {
        q: dto.q,
        root_id: dto.root_id,
        folder_prefix: dto.folder_prefix,
        tags_include: dto.tags_include,
        favorites_only: dto.favorites_only,
        limit: dto.limit.min(500), // hard cap: the UI never asks for the library
        offset: dto.offset,
        ..Default::default()
    };
    let rows = scout_search::search(&guard.conn, &q).map_err(|e| e.to_string())?;
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok(PageDto {
        rows: rows
            .into_iter()
            .map(|r| RowDto {
                id: r.id,
                rel_path: r.rel_path,
                name: r.name,
                ext: r.ext,
                duration_ms: r.duration_ms,
                bpm: r.bpm,
                favorite: r.favorite,
            })
            .collect(),
        total,
    })
}

#[derive(Debug, Serialize)]
pub struct TagDto {
    pub tag_id: i64,
    pub name: String,
    pub source: String,
    pub detail: Option<String>,
    pub confidence: Option<f32>,
    pub state: String,
}

#[tauri::command]
pub fn file_detail(state: State<AppState>, file_id: i64) -> Result<Vec<TagDto>, String> {
    let guard = state.db.lock().map_err(|e| e.to_string())?;
    let mut st = guard
        .conn
        .prepare(
            "SELECT t.id, t.name, ft.source, ft.source_detail, ft.confidence, ft.state
             FROM file_tags ft JOIN tags t ON t.id = ft.tag_id
             WHERE ft.file_id = ?1 AND ft.state != 'dismissed'
             ORDER BY ft.source != 'manual', t.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([file_id], |r| {
            Ok(TagDto {
                tag_id: r.get(0)?,
                name: r.get(1)?,
                source: r.get(2)?,
                detail: r.get(3)?,
                confidence: r.get(4)?,
                state: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn set_favorite(state: State<AppState>, file_id: i64, on: bool) -> Result<(), String> {
    let guard = state.db.lock().map_err(|e| e.to_string())?;
    if on {
        guard
            .conn
            .execute("INSERT OR IGNORE INTO favorites(file_id) VALUES (?1)", [file_id])
            .map_err(|e| e.to_string())?;
    } else {
        guard
            .conn
            .execute("DELETE FROM favorites WHERE file_id = ?1", [file_id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn dismiss_tag(state: State<AppState>, file_id: i64, tag_id: i64) -> Result<(), String> {
    let guard = state.db.lock().map_err(|e| e.to_string())?;
    scout_db::dismiss_tag(&guard.conn, file_id, tag_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn confirm_tag(state: State<AppState>, file_id: i64, tag_id: i64) -> Result<(), String> {
    let guard = state.db.lock().map_err(|e| e.to_string())?;
    scout_db::confirm_tag(&guard.conn, file_id, tag_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_scan(app: AppHandle, state: State<AppState>, root_id: i64) -> Result<(), String> {
    let root_path: String = {
        let guard = state.db.lock().map_err(|e| e.to_string())?;
        guard
            .conn
            .query_row("SELECT path FROM roots WHERE id=?1", [root_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
    };
    let (tx, rx) = crossbeam::channel::unbounded();
    let handle = scout_scanner::scan_root(
        state.db_path.clone(),
        root_id,
        PathBuf::from(root_path),
        Default::default(),
        tx,
    );
    *state.scan.lock().map_err(|e| e.to_string())? = Some(handle);
    std::thread::spawn(move || {
        for ev in rx {
            // frontend: listen('scan://progress', ...) — events are the ONLY
            // way progress reaches the UI; no polling the DB.
            let _ = app.emit("scan://progress", format!("{ev:?}"));
        }
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_scan(state: State<AppState>) -> Result<(), String> {
    if let Some(h) = state.scan.lock().map_err(|e| e.to_string())?.as_ref() {
        h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

/// ids -> absolute paths for drag-out. Network roots get copied to a temp
/// staging dir FIRST (drag-rs issue #72 = crash on SMB paths).
#[tauri::command]
pub fn paths_for_ids(state: State<AppState>, ids: Vec<i64>) -> Result<Vec<String>, String> {
    let guard = state.db.lock().map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(ids.len());
    let mut st = guard
        .conn
        .prepare("SELECT r.path, f.rel_path FROM files f JOIN roots r ON r.id=f.root_id WHERE f.id=?1")
        .map_err(|e| e.to_string())?;
    for id in ids {
        let row: rusqlite::Result<(String, String)> =
            st.query_row([id], |r| Ok((r.get(0)?, r.get(1)?)));
        if let Ok((root, rel)) = row {
            let full = PathBuf::from(root).join(rel);
            // TODO(M5): if full starts with a configured network root, stage
            // to std::env::temp_dir() and push the staged path instead.
            out.push(full.to_string_lossy().to_string());
        }
    }
    Ok(out)
}
