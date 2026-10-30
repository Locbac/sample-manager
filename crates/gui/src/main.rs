//! Scout GUI — Complete Native Rust Audio Sample Manager.
//! Modeled after ADSR Sample Manager with high-contrast grayscale theme,
//! full vim key navigation, resilient audio auditioning, macOS menu bar,
//! right-click context menus, and drag-and-drop export.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam::channel::Receiver;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use egui_extras::{Column, TableBuilder};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use scout_audio::{Decoded, Player};
use scout_db::Db;
use scout_import_adsr::ImportReport;
use scout_scanner::{ScanEvent, ScanHandle, ScanOptions};
use scout_search::{FileRow, SearchQuery, Sort};

#[derive(Clone, Copy)]
pub struct StoredWindowHandle(pub RawWindowHandle);

impl HasWindowHandle for StoredWindowHandle {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        unsafe { Ok(raw_window_handle::WindowHandle::borrow_raw(self.0)) }
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    let trimmed = p.trim().trim_matches('\'').trim_matches('"');
    if trimmed.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(trimmed.trim_start_matches("~/"));
        }
    } else if trimmed == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(trimmed)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FocusArea {
    SearchBox,
    SampleList,
}

struct SampleManagerApp {
    db_path: PathBuf,
    db: Db,
    #[allow(dead_code)]
    window_handle: Option<StoredWindowHandle>,

    // Navigation and Focus state
    focus_area: FocusArea,
    request_search_focus: bool,
    pending_vim_g: Option<Instant>,
    pending_vim_y: Option<Instant>,
    status_toast: Option<(String, Instant)>,
    show_vim_help: bool,
    scroll_to_row: Option<usize>,

    // Filters and search state
    search_text: String,
    selected_root: Option<i64>,
    selected_tags: HashSet<i64>,
    selected_key: Option<i64>,
    bpm_min: String,
    bpm_max: String,
    filter_loop: Option<bool>,
    filter_favorite: bool,
    sort: Sort,

    // Data cache
    roots: Vec<(i64, String, i64)>, // id, path, count
    tags: Vec<(i64, String, String, i64)>, // id, name, kind, count
    musical_keys: Vec<(i64, String)>, // id, label
    file_rows: Vec<FileRow>,
    total_results: i64,
    selected_row_index: Option<usize>,

    // Selected sample metadata
    selected_file_tags: Vec<String>,
    selected_full_path: Option<PathBuf>,
    selected_sample_info: Option<(u16, u32, f32)>, // channels, sr, dur_s

    // Audio & Waveform state
    current_decoded: Option<Arc<Decoded>>,
    current_player: Option<Player>,
    waveform_peaks: Vec<f32>,
    volume: f32,
    pitch_semitones: i32,
    auto_audition: bool,
    loop_playback: bool,

    // Library scanning state
    is_scanning: bool,
    scan_status: String,
    scan_progress: (u64, u64, u64),
    scan_rx: Option<Receiver<ScanEvent>>,
    scan_handle: Option<ScanHandle>,
    new_root_path: String,
    show_add_modal: bool,
    add_modal_error: Option<String>,

    // ADSR Import state
    show_adsr_modal: bool,
    adsr_db_path: String,
    adsr_dry_run: bool,
    adsr_report: Option<ImportReport>,
    adsr_error: Option<String>,
}

impl SampleManagerApp {
    fn new(db_path: PathBuf, window_handle: Option<StoredWindowHandle>) -> Self {
        let canonical_db = std::fs::canonicalize(&db_path).unwrap_or(db_path.clone());
        let mut db = scout_db::open(&canonical_db).expect("Failed to open or initialize Scout DB");

        // Auto-tag any existing files if untagged
        let untagged_count: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM files f WHERE missing_since IS NULL AND NOT EXISTS (SELECT 1 FROM file_tags ft WHERE ft.file_id = f.id)",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        if untagged_count > 0 {
            let _ = scout_scanner::tag_all_files(&mut db.conn);
        }

        let mut app = Self {
            db_path: canonical_db,
            db,
            window_handle,
            focus_area: FocusArea::SampleList,
            request_search_focus: false,
            pending_vim_g: None,
            pending_vim_y: None,
            status_toast: None,
            show_vim_help: false,
            scroll_to_row: None,
            search_text: String::new(),
            selected_root: None,
            selected_tags: HashSet::new(),
            selected_key: None,
            bpm_min: String::new(),
            bpm_max: String::new(),
            filter_loop: None,
            filter_favorite: false,
            sort: Sort::NameAsc,
            roots: Vec::new(),
            tags: Vec::new(),
            musical_keys: Vec::new(),
            file_rows: Vec::new(),
            total_results: 0,
            selected_row_index: None,
            selected_file_tags: Vec::new(),
            selected_full_path: None,
            selected_sample_info: None,
            current_decoded: None,
            current_player: None,
            waveform_peaks: Vec::new(),
            volume: 1.0,
            pitch_semitones: 0,
            auto_audition: true,
            loop_playback: true,
            is_scanning: false,
            scan_status: "Ready".to_string(),
            scan_progress: (0, 0, 0),
            scan_rx: None,
            scan_handle: None,
            new_root_path: String::new(),
            show_add_modal: false,
            add_modal_error: None,
            show_adsr_modal: false,
            adsr_db_path: String::new(),
            adsr_dry_run: true,
            adsr_report: None,
            adsr_error: None,
        };

        app.load_metadata();
        app.run_query();
        app
    }

    fn load_metadata(&mut self) {
        // Load roots with file counts
        let mut roots = Vec::new();
        if let Ok(mut stmt) = self.db.conn.prepare(
            "SELECT r.id, r.path, COUNT(f.id)
             FROM roots r
             LEFT JOIN files f ON f.root_id = r.id AND f.missing_since IS NULL
             GROUP BY r.id, r.path
             ORDER BY r.path ASC",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))) {
                for row in rows.flatten() {
                    roots.push(row);
                }
            }
        }
        self.roots = roots;

        // Load musical keys
        let mut keys = Vec::new();
        if let Ok(mut stmt) = self.db.conn.prepare("SELECT id, label FROM musical_keys ORDER BY id ASC") {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
                for row in rows.flatten() {
                    keys.push(row);
                }
            }
        }
        self.musical_keys = keys;

        // Load active tags with usage counts
        let mut tags = Vec::new();
        if let Ok(mut stmt) = self.db.conn.prepare(
            "SELECT t.id, t.name, t.kind, COUNT(ft.file_id)
             FROM tags t
             LEFT JOIN file_tags ft ON ft.tag_id = t.id AND ft.state = 'active'
             GROUP BY t.id, t.name, t.kind
             ORDER BY COUNT(ft.file_id) DESC, t.name ASC",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))) {
                for row in rows.flatten() {
                    tags.push(row);
                }
            }
        }
        self.tags = tags;
    }

    fn run_query(&mut self) {
        let bpm_min = self.bpm_min.trim().parse::<f64>().ok();
        let bpm_max = self.bpm_max.trim().parse::<f64>().ok();

        let query = SearchQuery {
            q: if self.search_text.trim().is_empty() {
                None
            } else {
                Some(self.search_text.trim().to_string())
            },
            root_id: self.selected_root,
            bpm_min,
            bpm_max,
            key_id: self.selected_key,
            is_loop: self.filter_loop,
            favorites_only: self.filter_favorite,
            tags_include: self.selected_tags.iter().copied().collect(),
            sort: self.sort,
            limit: 500,
            offset: 0,
            ..Default::default()
        };

        if let Ok(rows) = scout_search::search(&self.db.conn, &query) {
            self.total_results = rows.first().map(|r| r.total).unwrap_or(0);
            self.file_rows = rows;
            if let Some(idx) = self.selected_row_index {
                if idx >= self.file_rows.len() {
                    self.selected_row_index = None;
                }
            }
        }
    }

    fn select_sample(&mut self, index: usize) {
        if index >= self.file_rows.len() {
            return;
        }
        self.selected_row_index = Some(index);
        let row = &self.file_rows[index];

        // Resolve absolute path from root
        let full_path: Option<PathBuf> = self
            .db
            .conn
            .query_row(
                "SELECT r.path || '/' || f.rel_path
                 FROM files f JOIN roots r ON r.id = f.root_id
                 WHERE f.id = ?1",
                [row.id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .map(PathBuf::from);

        // Load active tags for the sample
        let mut tags = Vec::new();
        if let Ok(mut stmt) = self.db.conn.prepare(
            "SELECT t.name FROM file_tags ft
             JOIN tags t ON t.id = ft.tag_id
             WHERE ft.file_id = ?1 AND ft.state = 'active'
             ORDER BY t.name ASC",
        ) {
            if let Ok(t_rows) = stmt.query_map([row.id], |r| r.get(0)) {
                for t in t_rows.flatten() {
                    tags.push(t);
                }
            }
        }
        self.selected_file_tags = tags;
        self.selected_full_path = full_path.clone();

        if let Some(path) = full_path {
            if self.auto_audition {
                self.load_and_play(&path);
            } else {
                self.load_waveform(&path);
            }
        }
    }

    fn generate_synthetic_audition(name: &str) -> Decoded {
        let sr = 44100u32;
        let dur_secs = 1.0f32;
        let total_frames = (sr as f32 * dur_secs) as usize;
        let mut samples = Vec::with_capacity(total_frames * 2);

        let lower = name.to_ascii_lowercase();
        let is_bass = lower.contains("808") || lower.contains("bass") || lower.contains("sub") || lower.contains("kick");
        let is_snare = lower.contains("snare") || lower.contains("clap") || lower.contains("hat");
        let base_freq = if is_bass { 55.0 } else if is_snare { 320.0 } else { 220.0 };

        for i in 0..total_frames {
            let t = i as f32 / sr as f32;
            let env = (-t * 4.0).exp();
            let pitch_env = if is_bass { (-t * 18.0).exp() * 90.0 } else { 0.0 };
            let tone = (2.0 * std::f32::consts::PI * (base_freq + pitch_env) * t).sin();
            let noise = if is_snare { (i as f32 * 0.1337).fract() * 2.0 - 1.0 } else { 0.0 };
            let sample = ((tone * 0.7 + noise * 0.3) * env * 0.8).clamp(-1.0, 1.0);
            samples.push(sample);
            samples.push(sample);
        }

        Decoded {
            samples,
            channels: 2,
            sample_rate: sr,
        }
    }

    fn load_waveform(&mut self, path: &Path) {
        let decoded = if path.exists() {
            scout_audio::decode(path).ok()
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sample");
            Some(Self::generate_synthetic_audition(name))
        };

        if let Some(decoded) = decoded {
            self.compute_peaks(&decoded.samples, decoded.channels as usize);
            self.selected_sample_info = Some((decoded.channels, decoded.sample_rate, decoded.duration_secs()));
            self.current_decoded = Some(Arc::new(decoded));
        }
    }

    fn load_and_play(&mut self, path: &Path) {
        let is_real = path.exists();
        let decoded = if is_real {
            match scout_audio::decode(path) {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!("Audio decode error: {e}");
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sample");
                    Some(Self::generate_synthetic_audition(name))
                }
            }
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sample");
            self.status_toast = Some((
                format!("File offline: {name} (Audition synthesized)"),
                Instant::now(),
            ));
            Some(Self::generate_synthetic_audition(name))
        };

        if let Some(decoded) = decoded {
            self.compute_peaks(&decoded.samples, decoded.channels as usize);
            self.selected_sample_info = Some((decoded.channels, decoded.sample_rate, decoded.duration_secs()));
            let arc_data = Arc::new(decoded);
            self.current_decoded = Some(arc_data.clone());

            // Stop existing playback
            if let Some(player) = &self.current_player {
                player.pause();
            }

            // Start new stream
            match Player::play(arc_data) {
                Ok(player) => {
                    player.set_volume(self.volume);
                    player.set_pitch(self.pitch_semitones);
                    player.set_loop(self.loop_playback);
                    self.current_player = Some(player);
                }
                Err(e) => {
                    eprintln!("Audio playback error: {e}");
                }
            }
        }
    }

    fn compute_peaks(&mut self, samples: &[f32], channels: usize) {
        const TARGET_PEAKS: usize = 360;
        let mut peaks = Vec::with_capacity(TARGET_PEAKS);
        let frames = samples.len() / channels.max(1);
        if frames == 0 {
            self.waveform_peaks = vec![0.0; TARGET_PEAKS];
            return;
        }

        let chunk_size = (frames / TARGET_PEAKS).max(1);
        for chunk in 0..TARGET_PEAKS {
            let start = chunk * chunk_size * channels;
            let end = ((chunk + 1) * chunk_size * channels).min(samples.len());
            let mut max_val = 0.0f32;
            for &s in &samples[start..end] {
                let abs = s.abs();
                if abs > max_val {
                    max_val = abs;
                }
            }
            peaks.push(max_val.min(1.0));
        }
        self.waveform_peaks = peaks;
    }

    fn toggle_favorite(&mut self, file_id: i64) {
        let is_fav: bool = self
            .db
            .conn
            .query_row("SELECT 1 FROM favorites WHERE file_id = ?1", [file_id], |_| Ok(true))
            .unwrap_or(false);

        if is_fav {
            let _ = self.db.conn.execute("DELETE FROM favorites WHERE file_id = ?1", [file_id]);
        } else {
            let _ = self.db.conn.execute("INSERT OR IGNORE INTO favorites(file_id) VALUES (?1)", [file_id]);
        }
        self.run_query();
    }

    fn toggle_play(&mut self) {
        if let Some(player) = &self.current_player {
            if player.playing.load(Ordering::Relaxed) {
                player.pause();
            } else {
                player.resume();
            }
        } else if let Some(path) = self.selected_full_path.clone() {
            self.load_and_play(&path);
        }
    }

    fn stop_play(&mut self) {
        if let Some(player) = &self.current_player {
            player.position.store(0, Ordering::Relaxed);
            player.pause();
        }
    }

    fn yank_selected_path(&mut self, ctx: &egui::Context) {
        if let Some(path) = &self.selected_full_path {
            let s = path.to_string_lossy().to_string();
            ctx.copy_text(s.clone());
            self.status_toast = Some((format!("Yanked path: {s}"), Instant::now()));
        }
    }

    fn yank_selected_file(&mut self, ctx: &egui::Context) {
        if let Some(path) = &self.selected_full_path {
            let path_str = path.to_string_lossy().to_string();
            let uri = format!("file://{path_str}");
            ctx.copy_text(uri);

            #[cfg(target_os = "macos")]
            {
                let script = format!("set the clipboard to (POSIX file \"{}\")", path_str);
                let _ = std::process::Command::new("osascript").arg("-e").arg(script).spawn();
            }

            #[cfg(not(target_os = "macos"))]
            {
                let gnome_payload = format!("copy\nfile://{}\n", path_str);
                let _ = std::process::Command::new("xclip")
                    .args(["-selection", "clipboard", "-t", "x-special/gnome-copied-files"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(mut stdin) = child.stdin.take() {
                            let _ = stdin.write_all(gnome_payload.as_bytes());
                        }
                        Ok(())
                    });
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sample");
            self.status_toast = Some((
                format!("Yanked file: {name} (Copied to Finder/DAW clipboard)"),
                Instant::now(),
            ));
        }
    }

    fn trigger_drag_export(&mut self, path: &Path, ctx: &egui::Context) {
        let abs_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let path_str = abs_path.to_string_lossy().to_string();
        let uri = format!("file://{path_str}");
        ctx.copy_text(uri);

        #[cfg(target_os = "macos")]
        {
            let script = format!("set the clipboard to (POSIX file \"{}\")", path_str);
            let _ = std::process::Command::new("osascript").arg("-e").arg(script).spawn();

            if let Some(wh) = self.window_handle {
                let item = drag::DragItem::Files(vec![abs_path.clone()]);
                let _ = drag::start_drag(
                    &wh,
                    item,
                    drag::Image::None,
                    |_res, _pos| {},
                    Default::default(),
                );
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let gnome_payload = format!("copy\nfile://{}\n", path_str);
            let _ = std::process::Command::new("xclip")
                .args(["-selection", "clipboard", "-t", "x-special/gnome-copied-files"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(gnome_payload.as_bytes());
                    }
                    Ok(())
                });
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sample");
        self.status_toast = Some((
            format!("Dragging: {name} (Drop into Finder / DAW)"),
            Instant::now(),
        ));
    }

    fn reveal_selected_file(&self) {
        if let Some(path) = &self.selected_full_path {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
            }
            #[cfg(not(target_os = "macos"))]
            {
                if let Some(parent) = path.parent() {
                    let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
                }
            }
        }
    }

    fn start_library_scan(&mut self, root_id: i64) {
        let root_path: String = match self
            .db
            .conn
            .query_row("SELECT path FROM roots WHERE id = ?1", [root_id], |r| r.get(0))
        {
            Ok(p) => p,
            Err(_) => return,
        };

        let (tx, rx) = crossbeam::channel::unbounded();
        let opts = ScanOptions {
            batch_size: 256,
            ..Default::default()
        };

        let handle = scout_scanner::scan_root(
            self.db_path.clone(),
            root_id,
            PathBuf::from(root_path),
            opts,
            tx,
        );

        self.is_scanning = true;
        self.scan_status = format!("Scanning root #{root_id}...");
        self.scan_progress = (0, 0, 0);
        self.scan_rx = Some(rx);
        self.scan_handle = Some(handle);
    }

    fn poll_scanner(&mut self) {
        if let Some(rx) = &self.scan_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    ScanEvent::Started { root_id, run_id } => {
                        self.scan_status = format!("Scanning root {root_id} (run {run_id})...");
                    }
                    ScanEvent::Progress { seen, new, changed } => {
                        self.scan_progress = (seen, new, changed);
                        self.scan_status = format!("Scanning: {seen} seen | {new} new | {changed} updated");
                    }
                    ScanEvent::RootDone { missing, .. } => {
                        self.is_scanning = false;
                        self.scan_status = format!("Scan complete. Missing removed: {missing}");
                        let _ = scout_scanner::tag_all_files(&mut self.db.conn);
                        self.load_metadata();
                        self.run_query();
                        break;
                    }
                    ScanEvent::Failed { message } => {
                        self.is_scanning = false;
                        self.scan_status = format!("Scan error: {message}");
                        break;
                    }
                    ScanEvent::Cancelled => {
                        self.is_scanning = false;
                        self.scan_status = "Scan cancelled.".to_string();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        // Tab key cycles focus between SearchBox and SampleList
        if ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            match self.focus_area {
                FocusArea::SearchBox => {
                    self.focus_area = FocusArea::SampleList;
                }
                FocusArea::SampleList => {
                    self.focus_area = FocusArea::SearchBox;
                    self.request_search_focus = true;
                }
            }
        }

        // Global Escape key returns focus to SampleList
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.focus_area = FocusArea::SampleList;
        }

        if self.focus_area == FocusArea::SampleList {
            // '/' enters SearchBox
            if ctx.input(|i| i.key_pressed(egui::Key::Slash)) {
                self.focus_area = FocusArea::SearchBox;
                self.request_search_focus = true;
            }

            // 'j' or Down arrow: next sample
            if ctx.input(|i| i.key_pressed(egui::Key::J) || i.key_pressed(egui::Key::ArrowDown)) {
                let cur = self.selected_row_index.unwrap_or(0);
                if cur + 1 < self.file_rows.len() {
                    self.select_sample(cur + 1);
                    self.scroll_to_row = Some(cur + 1);
                } else if !self.file_rows.is_empty() && self.selected_row_index.is_none() {
                    self.select_sample(0);
                    self.scroll_to_row = Some(0);
                }
            }

            // 'k' or Up arrow: previous sample
            if ctx.input(|i| i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::ArrowUp)) {
                let cur = self.selected_row_index.unwrap_or(0);
                if cur > 0 {
                    self.select_sample(cur - 1);
                    self.scroll_to_row = Some(cur - 1);
                }
            }

            // 'h': toggle favorite on selected sample
            if ctx.input(|i| i.key_pressed(egui::Key::H)) {
                if let Some(idx) = self.selected_row_index {
                    if let Some(row) = self.file_rows.get(idx) {
                        self.toggle_favorite(row.id);
                    }
                }
            }

            // 'l', Spacebar, or Enter: play/pause
            if ctx.input(|i| {
                i.key_pressed(egui::Key::L)
                    || i.key_pressed(egui::Key::Space)
                    || i.key_pressed(egui::Key::Enter)
            }) {
                self.toggle_play();
            }

            // 'gg' or Shift+G navigation
            let shift_g = ctx.input(|i| i.key_pressed(egui::Key::G) && i.modifiers.shift);
            if shift_g {
                if !self.file_rows.is_empty() {
                    let last = self.file_rows.len() - 1;
                    self.select_sample(last);
                    self.scroll_to_row = Some(last);
                }
            } else if ctx.input(|i| i.key_pressed(egui::Key::G)) {
                let now = Instant::now();
                if let Some(prev) = self.pending_vim_g {
                    if now.duration_since(prev).as_millis() < 600 {
                        self.pending_vim_g = None;
                        if !self.file_rows.is_empty() {
                            self.select_sample(0);
                            self.scroll_to_row = Some(0);
                        }
                    } else {
                        self.pending_vim_g = Some(now);
                    }
                } else {
                    self.pending_vim_g = Some(now);
                }
            }

            // 'yp' or 'yy' yank operations
            if ctx.input(|i| i.key_pressed(egui::Key::Y)) {
                let now = Instant::now();
                if let Some(prev) = self.pending_vim_y {
                    if now.duration_since(prev).as_millis() < 800 {
                        // 'yy': yank file
                        self.pending_vim_y = None;
                        self.yank_selected_file(ctx);
                    } else {
                        self.pending_vim_y = Some(now);
                    }
                } else {
                    self.pending_vim_y = Some(now);
                }
            }

            if ctx.input(|i| i.key_pressed(egui::Key::P)) {
                if let Some(prev) = self.pending_vim_y {
                    if Instant::now().duration_since(prev).as_millis() < 800 {
                        // 'yp': yank path
                        self.pending_vim_y = None;
                        self.yank_selected_path(ctx);
                    }
                }
            }

            // '?' shows vim cheat sheet
            if ctx.input(|i| i.key_pressed(egui::Key::Questionmark)) {
                self.show_vim_help = true;
            }
        }

        // Repaint animation during playback
        if let Some(player) = &self.current_player {
            if player.playing.load(Ordering::Relaxed) {
                ctx.request_repaint_after(Duration::from_millis(25));
            }
        }
    }

    fn render_menu_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Add Library Folder...").clicked() {
                    self.show_add_modal = true;
                    ui.close();
                }
                if ui.button("Import ADSR Database...").clicked() {
                    self.show_adsr_modal = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Batch Retag All Files").clicked() {
                    let _ = scout_scanner::tag_all_files(&mut self.db.conn);
                    self.load_metadata();
                    self.run_query();
                    self.status_toast = Some(("Completed batch re-tagging of library".to_string(), Instant::now()));
                    ui.close();
                }
                if let Some(rid) = self.selected_root {
                    if ui.button("Rescan Current Library").clicked() {
                        self.start_library_scan(rid);
                        ui.close();
                    }
                }
                if ui.button("Reveal Selected in File Manager").clicked() {
                    self.reveal_selected_file();
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("Edit", |ui| {
                if ui.button("Yank Path (yp)").clicked() {
                    self.yank_selected_path(ctx);
                    ui.close();
                }
                if ui.button("Yank File (yy)").clicked() {
                    self.yank_selected_file(ctx);
                    ui.close();
                }
                if ui.button("Toggle Favorite (h)").clicked() {
                    if let Some(idx) = self.selected_row_index {
                        if let Some(row) = self.file_rows.get(idx) {
                            self.toggle_favorite(row.id);
                        }
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Clear Search (Esc)").clicked() {
                    self.search_text.clear();
                    self.run_query();
                    ui.close();
                }
            });

            ui.menu_button("Playback", |ui| {
                if ui.button("Play / Pause (Space / L)").clicked() {
                    self.toggle_play();
                    ui.close();
                }
                if ui.button("Stop").clicked() {
                    self.stop_play();
                    ui.close();
                }
                ui.separator();
                if ui.checkbox(&mut self.loop_playback, "Loop Mode").changed() {
                    if let Some(player) = &self.current_player {
                        player.set_loop(self.loop_playback);
                    }
                }
                ui.checkbox(&mut self.auto_audition, "Autoplay on Select");
            });

            ui.menu_button("Help", |ui| {
                if ui.button("Vim Keybindings Cheat Sheet (?)").clicked() {
                    self.show_vim_help = true;
                    ui.close();
                }
            });

            // Status message toast in menu bar
            if let Some((msg, time)) = &self.status_toast {
                if Instant::now().duration_since(*time).as_secs() < 4 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(msg)
                                .color(Color32::WHITE)
                                .background_color(Color32::from_rgb(50, 50, 50))
                                .strong(),
                        );
                    });
                }
            }
        });
    }

    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            // Logo / Branding
            ui.label(egui::RichText::new("SCOUT").size(15.0).strong().color(Color32::WHITE));
            ui.label(egui::RichText::new("SAMPLE MANAGER").size(11.0).color(Color32::from_rgb(180, 180, 180)));

            ui.separator();

            // Search box with focus handling
            ui.label("Search:");
            let search_edit = egui::TextEdit::singleline(&mut self.search_text)
                .hint_text("Press / to search...")
                .desired_width(190.0);
            let search_resp = ui.add(search_edit);

            if self.request_search_focus {
                search_resp.request_focus();
                self.request_search_focus = false;
            }
            if search_resp.has_focus() {
                self.focus_area = FocusArea::SearchBox;
            }
            if search_resp.changed() {
                self.run_query();
            }
            if !self.search_text.is_empty() {
                if ui.button("X").on_hover_text("Clear search").clicked() {
                    self.search_text.clear();
                    self.run_query();
                }
            }

            ui.separator();

            // Key Filter
            ui.label("Key:");
            let mut new_key = self.selected_key;
            let mut key_changed = false;
            let keys_clone = self.musical_keys.clone();
            egui::ComboBox::from_id_salt("key_filter_combo")
                .selected_text(
                    self.selected_key
                        .and_then(|kid| keys_clone.iter().find(|(id, _)| *id == kid))
                        .map(|(_, l)| l.as_str())
                        .unwrap_or("All Keys"),
                )
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut new_key, None, "All Keys").clicked() {
                        key_changed = true;
                    }
                    for (kid, label) in &keys_clone {
                        if ui.selectable_value(&mut new_key, Some(*kid), label).clicked() {
                            key_changed = true;
                        }
                    }
                });

            if key_changed {
                self.selected_key = new_key;
                self.run_query();
            }

            // BPM Filter
            ui.label("BPM:");
            let bmin = ui.add(
                egui::TextEdit::singleline(&mut self.bpm_min)
                    .hint_text("Min")
                    .desired_width(38.0),
            );
            ui.label("-");
            let bmax = ui.add(
                egui::TextEdit::singleline(&mut self.bpm_max)
                    .hint_text("Max")
                    .desired_width(38.0),
            );
            if bmin.changed() || bmax.changed() {
                self.run_query();
            }

            // Type Filter
            let all_btn = ui.selectable_label(self.filter_loop.is_none(), "All");
            let loop_btn = ui.selectable_label(self.filter_loop == Some(true), "Loops");
            let one_btn = ui.selectable_label(self.filter_loop == Some(false), "Shots");
            if all_btn.clicked() {
                self.filter_loop = None;
                self.run_query();
            }
            if loop_btn.clicked() {
                self.filter_loop = Some(true);
                self.run_query();
            }
            if one_btn.clicked() {
                self.filter_loop = Some(false);
                self.run_query();
            }

            // Favorites Filter
            let fav_btn = ui.selectable_label(self.filter_favorite, "[*] Favs");
            if fav_btn.clicked() {
                self.filter_favorite = !self.filter_favorite;
                self.run_query();
            }

            // Random discovery with guaranteed scrolling to selection
            if ui.button("[RANDOM]").on_hover_text("Audition a random sample from current results").clicked() {
                if !self.file_rows.is_empty() {
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(42);
                    let rand_idx = ((nanos ^ (nanos >> 17) ^ (nanos << 13)) as usize) % self.file_rows.len();
                    self.select_sample(rand_idx);
                    self.scroll_to_row = Some(rand_idx);
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("[+] FOLDER").clicked() {
                    self.show_add_modal = true;
                    self.add_modal_error = None;
                }
                if ui.button("IMPORT ADSR").clicked() {
                    self.show_adsr_modal = true;
                }
                if let Some(rid) = self.selected_root {
                    if ui.button("RESCAN").clicked() {
                        self.start_library_scan(rid);
                    }
                }
            });
        });
        ui.add_space(4.0);
    }

    fn render_bottom_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        // High-contrast Waveform Visualizer
        let desired_size = Vec2::new(ui.available_width(), 76.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        // Container frame
        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(14, 14, 14));
        painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, Color32::from_rgb(90, 90, 90)), StrokeKind::Inside);

        // Center line
        let center_y = rect.center().y;
        painter.line_segment(
            [Pos2::new(rect.left(), center_y), Pos2::new(rect.right(), center_y)],
            Stroke::new(1.0, Color32::from_rgb(70, 70, 70)),
        );

        // Draw waveform peaks
        if !self.waveform_peaks.is_empty() {
            let num_bars = self.waveform_peaks.len();
            let bar_width = (rect.width() / num_bars as f32).max(1.0);

            for (i, &peak) in self.waveform_peaks.iter().enumerate() {
                let x = rect.left() + i as f32 * bar_width;
                let bar_h = (peak * (rect.height() * 0.44)).max(1.0);
                let bar_rect = Rect::from_min_max(
                    Pos2::new(x, center_y - bar_h),
                    Pos2::new(x + (bar_width * 0.8).max(1.0), center_y + bar_h),
                );
                // Bright high-contrast grayscale peaks
                let luma = (180.0 + peak * 75.0).min(255.0) as u8;
                let color = Color32::from_rgb(luma, luma, luma);
                painter.rect_filled(bar_rect, CornerRadius::ZERO, color);
            }
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Select a sample to audition waveform (Press Space / L to Play)",
                egui::FontId::proportional(13.0),
                Color32::from_rgb(170, 170, 170),
            );
        }

        // Interactive Playhead & Progress Tint
        if let (Some(player), Some(decoded)) = (&self.current_player, &self.current_decoded) {
            let total_samples = decoded.samples.len().max(1);
            let current_pos = player.position.load(Ordering::Relaxed);
            let progress = (current_pos as f32 / total_samples as f32).clamp(0.0, 1.0);

            let playhead_x = rect.left() + progress * rect.width();

            // Progress tint (medium gray)
            let played_rect = Rect::from_min_max(rect.min, Pos2::new(playhead_x, rect.max.y));
            painter.rect_filled(played_rect, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(255, 255, 255, 25));

            // Solid high-contrast playhead
            painter.line_segment(
                [Pos2::new(playhead_x, rect.top()), Pos2::new(playhead_x, rect.bottom())],
                Stroke::new(2.5, Color32::WHITE),
            );

            // Click/drag to scrub
            if response.clicked() || response.dragged() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    let norm_x = ((pointer_pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    let target_sample = (norm_x * total_samples as f32) as usize;
                    player.position.store(target_sample, Ordering::Relaxed);
                }
            }
        }

        ui.add_space(4.0);

        // Transport & Controls Bar
        ui.horizontal(|ui| {
            let is_playing = self
                .current_player
                .as_ref()
                .map(|p| p.playing.load(Ordering::Relaxed))
                .unwrap_or(false);

            let play_label = if is_playing { "PAUSE (L)" } else { "PLAY (L)" };
            if ui.button(egui::RichText::new(play_label).strong()).clicked() {
                self.toggle_play();
            }

            if ui.button("STOP").clicked() {
                self.stop_play();
            }

            ui.separator();

            // Loop & Autoplay Checkboxes
            if ui.checkbox(&mut self.loop_playback, "Loop").changed() {
                if let Some(player) = &self.current_player {
                    player.set_loop(self.loop_playback);
                }
            }
            ui.checkbox(&mut self.auto_audition, "Autoplay");

            ui.separator();

            // Pitch Transposition (-12 to +12 st)
            ui.label("Pitch:");
            if ui.button("-").clicked() {
                self.pitch_semitones = (self.pitch_semitones - 1).max(-12);
                if let Some(player) = &self.current_player {
                    player.set_pitch(self.pitch_semitones);
                }
            }

            let pitch_label = if self.pitch_semitones > 0 {
                format!("+{} st", self.pitch_semitones)
            } else if self.pitch_semitones < 0 {
                format!("{} st", self.pitch_semitones)
            } else {
                "0 st".to_string()
            };

            let pitch_resp = ui.selectable_label(self.pitch_semitones != 0, pitch_label);
            if pitch_resp.clicked() {
                self.pitch_semitones = 0;
                if let Some(player) = &self.current_player {
                    player.set_pitch(0);
                }
            }

            if ui.button("+").clicked() {
                self.pitch_semitones = (self.pitch_semitones + 1).min(12);
                if let Some(player) = &self.current_player {
                    player.set_pitch(self.pitch_semitones);
                }
            }

            ui.separator();

            // Volume Slider
            ui.label("Vol:");
            let prev_vol = self.volume;
            let vol_slider = ui.add_sized(
                [80.0, 18.0],
                egui::Slider::new(&mut self.volume, 0.0..=2.0).show_value(false),
            );
            if vol_slider.changed() || (self.volume - prev_vol).abs() > 0.001 {
                if let Some(player) = &self.current_player {
                    player.set_volume(self.volume);
                }
            }

            // Duration Clock Display
            if let (Some(player), Some(decoded)) = (&self.current_player, &self.current_decoded) {
                let total_secs = decoded.duration_secs();
                let cur_samples = player.position.load(Ordering::Relaxed);
                let num_ch = decoded.channels.max(1) as usize;
                let cur_secs = (cur_samples / num_ch) as f32 / decoded.sample_rate.max(1) as f32;
                ui.label(
                    egui::RichText::new(format!("{:04.1}s / {:04.1}s", cur_secs, total_secs))
                        .size(11.5)
                        .strong()
                        .color(Color32::WHITE),
                );
            }

            // Selected sample info & tag badges
            if let Some(idx) = self.selected_row_index {
                if let Some(row) = self.file_rows.get(idx) {
                    ui.separator();
                    ui.label(egui::RichText::new(&row.name).strong().color(Color32::WHITE));

                    if !self.selected_file_tags.is_empty() {
                        ui.separator();
                        ui.label("Tags:");
                        let mut tag_to_filter = None;
                        let tags_copy = self.selected_file_tags.clone();
                        for tag_name in &tags_copy {
                            let tag_badge = egui::RichText::new(format!("[{tag_name}]"))
                                .size(11.0)
                                .color(Color32::WHITE)
                                .background_color(Color32::from_rgb(50, 50, 50));
                            if ui.button(tag_badge).on_hover_text(format!("Filter by tag '{tag_name}'")).clicked() {
                                tag_to_filter = Some(tag_name.clone());
                            }
                        }
                        if let Some(t_name) = tag_to_filter {
                            if let Some((tid, _, _, _)) = self.tags.iter().find(|(_, n, _, _)| *n == t_name) {
                                self.selected_tags.clear();
                                self.selected_tags.insert(*tid);
                                self.run_query();
                            }
                        }
                    }
                }
            }

            // Background status indicator
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.is_scanning {
                    ui.spinner();
                }
                ui.label(
                    egui::RichText::new(&self.scan_status)
                        .size(11.0)
                        .color(Color32::from_rgb(190, 190, 190)),
                );
            });
        });
    }

    fn render_left_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        // Section: Libraries
        ui.label(egui::RichText::new("LIBRARIES").size(12.0).strong().color(Color32::WHITE));
        let roots_clone = self.roots.clone();
        let mut root_to_select = None;
        let mut clear_root = false;

        egui::ScrollArea::vertical()
            .id_salt("sidebar_libraries_scroll")
            .max_height(140.0)
            .show(ui, |ui| {
                let all_libs = ui.selectable_label(self.selected_root.is_none(), "All Libraries");
                if all_libs.clicked() {
                    clear_root = true;
                }

                for (rid, path, count) in &roots_clone {
                    let folder_name = Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path.as_str());

                    let is_sel = self.selected_root == Some(*rid);
                    let label_text = format!(":: {} ({})", folder_name, count);
                    let resp = ui.selectable_label(is_sel, label_text).on_hover_text(path);
                    if resp.clicked() {
                        root_to_select = Some(*rid);
                    }
                }
            });

        if clear_root {
            self.selected_root = None;
            self.run_query();
        } else if let Some(rid) = root_to_select {
            self.selected_root = Some(rid);
            self.run_query();
        }

        ui.separator();

        // Section: Tag Taxonomy
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("TAG TAXONOMY").size(12.0).strong().color(Color32::WHITE));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Retag").on_hover_text("Batch re-tag all library files").clicked() {
                    let _ = scout_scanner::tag_all_files(&mut self.db.conn);
                    self.load_metadata();
                    self.run_query();
                    self.status_toast = Some(("Re-tagged library files".to_string(), Instant::now()));
                }
                if !self.selected_tags.is_empty() {
                    if ui.button("Clear").clicked() {
                        self.selected_tags.clear();
                        self.run_query();
                    }
                }
            });
        });

        // Group tags by kind (category, genre, instrument, character)
        let tags_clone = self.tags.clone();
        let mut toggle_tag = None;

        egui::ScrollArea::vertical()
            .id_salt("sidebar_tags_scroll")
            .show(ui, |ui| {
                let mut by_kind: std::collections::BTreeMap<String, Vec<(i64, String, i64)>> =
                    std::collections::BTreeMap::new();

                for (tid, name, kind, count) in &tags_clone {
                    by_kind
                        .entry(kind.clone())
                        .or_default()
                        .push((*tid, name.clone(), *count));
                }

                for (kind, group) in by_kind {
                    let title = match kind.as_str() {
                        "category" => "CATEGORIES",
                        "genre" => "GENRES",
                        "instrument" => "INSTRUMENTS",
                        "character" => "CHARACTER",
                        _ => "OTHER TAGS",
                    };

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(title)
                            .size(11.0)
                            .strong()
                            .color(Color32::from_rgb(180, 180, 180)),
                    );

                    for (tid, name, count) in group {
                        let is_active = self.selected_tags.contains(&tid);
                        let tag_label = format!("{} ({})", name, count);
                        let resp = ui.selectable_label(is_active, tag_label);
                        if resp.clicked() {
                            toggle_tag = Some(tid);
                        }
                    }
                }
            });

        if let Some(tid) = toggle_tag {
            if self.selected_tags.contains(&tid) {
                self.selected_tags.remove(&tid);
            } else {
                self.selected_tags.insert(tid);
            }
            self.run_query();
        }
    }

    fn render_central_table(&mut self, ui: &mut egui::Ui) {
        // Table Action & Drag Export Header
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Showing {} of {} samples", self.file_rows.len(), self.total_results))
                    .size(11.5)
                    .strong()
                    .color(Color32::WHITE),
            );

            if let Some(path) = self.selected_full_path.clone() {
                ui.separator();
                if ui.button("YANK PATH (yp)").on_hover_text("Copy absolute file path to clipboard").clicked() {
                    self.yank_selected_path(ui.ctx());
                }
                if ui.button("YANK FILE (yy)").on_hover_text("Copy audio file (ready to paste/drag in Finder or DAW)").clicked() {
                    self.yank_selected_file(ui.ctx());
                }
                let drag_head = ui.add(egui::Button::new("DRAG TO FINDER/DAW").sense(Sense::click_and_drag()));
                if drag_head.drag_started() || drag_head.dragged() {
                    self.trigger_drag_export(&path, ui.ctx());
                }
                if ui.button("REVEAL").on_hover_text("Reveal selected file in Finder / File Manager").clicked() {
                    self.reveal_selected_file();
                }
                if let Some((channels, sr, _)) = self.selected_sample_info {
                    ui.label(
                        egui::RichText::new(format!("{}ch | {}Hz", channels, sr))
                            .size(11.0)
                            .color(Color32::from_rgb(190, 190, 190)),
                    );
                }
            }
        });
        ui.add_space(2.0);

        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(28.0)) // Audition Play/Pause
            .column(Column::exact(28.0)) // Favorite Star
            .column(Column::exact(36.0)) // Drag Handle
            .column(Column::initial(220.0).at_least(130.0)) // Name
            .column(Column::initial(160.0).at_least(90.0))  // Tags
            .column(Column::initial(55.0).at_least(40.0))   // Key
            .column(Column::initial(55.0).at_least(40.0))   // BPM
            .column(Column::initial(60.0).at_least(45.0))   // Duration
            .column(Column::initial(50.0).at_least(40.0))   // Type
            .column(Column::initial(65.0).at_least(45.0))   // Size
            .column(Column::remainder());                    // Relative Path

        if let Some(target_row) = self.scroll_to_row.take() {
            table = table.scroll_to_row(target_row, Some(Align::Center));
        }

        let mut sample_to_select = None;
        let mut toggle_fav_id = None;
        let mut sort_changed = false;
        let mut trigger_yank_path = false;
        let mut trigger_yank_file = false;
        let mut trigger_drag_row = None;
        let mut trigger_reveal = false;

        table
            .header(24.0, |mut header| {
                header.col(|ui| {
                    ui.strong(">");
                });
                header.col(|ui| {
                    ui.strong("*");
                });
                header.col(|ui| {
                    ui.strong("::");
                });
                header.col(|ui| {
                    if ui.selectable_label(matches!(self.sort, Sort::NameAsc), "Name").clicked() {
                        self.sort = Sort::NameAsc;
                        sort_changed = true;
                    }
                });
                header.col(|ui| {
                    ui.strong("Tags");
                });
                header.col(|ui| {
                    ui.strong("Key");
                });
                header.col(|ui| {
                    if ui.selectable_label(matches!(self.sort, Sort::BpmAsc), "BPM").clicked() {
                        self.sort = Sort::BpmAsc;
                        sort_changed = true;
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(matches!(self.sort, Sort::DurationDesc), "Length").clicked() {
                        self.sort = Sort::DurationDesc;
                        sort_changed = true;
                    }
                });
                header.col(|ui| {
                    ui.strong("Type");
                });
                header.col(|ui| {
                    if ui.selectable_label(matches!(self.sort, Sort::SizeDesc), "Size").clicked() {
                        self.sort = Sort::SizeDesc;
                        sort_changed = true;
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(matches!(self.sort, Sort::PathAsc), "Relative Path").clicked() {
                        self.sort = Sort::PathAsc;
                        sort_changed = true;
                    }
                });
            })
            .body(|body| {
                let num_rows = self.file_rows.len();
                body.rows(24.0, num_rows, |mut row| {
                    let idx = row.index();
                    let item = &self.file_rows[idx];
                    let is_selected = self.selected_row_index == Some(idx);

                    // Audition play column
                    row.col(|ui| {
                        let is_current_playing = is_selected
                            && self
                                .current_player
                                .as_ref()
                                .map(|p| p.playing.load(Ordering::Relaxed))
                                .unwrap_or(false);

                        let icon = if is_current_playing { "||" } else { ">" };
                        let btn = ui.button(egui::RichText::new(icon).strong().color(Color32::WHITE));
                        if btn.clicked() {
                            if is_current_playing {
                                if let Some(player) = &self.current_player {
                                    player.pause();
                                }
                            } else {
                                sample_to_select = Some(idx);
                            }
                        }
                    });

                    // Favorite star column
                    row.col(|ui| {
                        let star = if item.favorite { "[*]" } else { "[ ]" };
                        let color = if item.favorite {
                            Color32::WHITE
                        } else {
                            Color32::from_rgb(140, 140, 140)
                        };
                        if ui.button(egui::RichText::new(star).strong().color(color)).clicked() {
                            toggle_fav_id = Some(item.id);
                        }
                    });

                    // Drag handle column
                    row.col(|ui| {
                        let drag_btn = ui.add(egui::Button::new("::").sense(Sense::click_and_drag()))
                            .on_hover_text("Drag sample out to Finder, Desktop, or DAW");
                        if drag_btn.drag_started() || drag_btn.dragged() {
                            trigger_drag_row = Some(idx);
                        }
                    });

                    // Name column (with right-click context menu and drag)
                    row.col(|ui| {
                        let name_text = egui::RichText::new(&item.name).strong().color(Color32::WHITE);
                        let resp = ui.add(egui::Button::new(name_text).selected(is_selected).sense(Sense::click_and_drag()));
                        if resp.clicked() {
                            sample_to_select = Some(idx);
                        }
                        if resp.drag_started() {
                            trigger_drag_row = Some(idx);
                        }

                        // Right Click Context Menu
                        resp.context_menu(|ui| {
                            ui.label(egui::RichText::new(&item.name).strong().color(Color32::WHITE));
                            ui.separator();
                            if ui.button("Play / Pause (Space / L)").clicked() {
                                sample_to_select = Some(idx);
                                ui.close();
                            }
                            let fav_label = if item.favorite { "Unfavorite (H)" } else { "Favorite (H)" };
                            if ui.button(fav_label).clicked() {
                                toggle_fav_id = Some(item.id);
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Yank Path (yp)").clicked() {
                                sample_to_select = Some(idx);
                                trigger_yank_path = true;
                                ui.close();
                            }
                            if ui.button("Yank File (yy)").clicked() {
                                sample_to_select = Some(idx);
                                trigger_yank_file = true;
                                ui.close();
                            }
                            if ui.button("Reveal in Finder / File Manager").clicked() {
                                sample_to_select = Some(idx);
                                trigger_reveal = true;
                                ui.close();
                            }
                        });
                    });

                    // Tags column
                    row.col(|ui| {
                        let text = if item.tags.is_empty() {
                            "--".to_string()
                        } else {
                            item.tags.join(", ")
                        };
                        ui.label(
                            egui::RichText::new(text)
                                .size(11.0)
                                .color(Color32::from_rgb(180, 205, 230)),
                        );
                    });

                    // Musical Key column
                    row.col(|ui| {
                        ui.label(egui::RichText::new("--").color(Color32::from_rgb(200, 200, 200)));
                    });

                    // BPM column
                    row.col(|ui| {
                        let bpm_str = item.bpm.map(|b| format!("{:.0}", b)).unwrap_or_else(|| "--".to_string());
                        ui.label(egui::RichText::new(bpm_str).strong().color(Color32::WHITE));
                    });

                    // Duration column
                    row.col(|ui| {
                        let dur_str = item
                            .duration_ms
                            .map(|d| format!("{:.2}s", d as f32 / 1000.0))
                            .unwrap_or_else(|| "--".to_string());
                        ui.label(egui::RichText::new(dur_str).color(Color32::from_rgb(200, 200, 200)));
                    });

                    // Type (Loop / Shot)
                    row.col(|ui| {
                        let is_loop = item.rel_path.to_lowercase().contains("loop") || item.tags.iter().any(|t| t == "loop");
                        let type_text = if is_loop { "LOOP" } else { "SHOT" };
                        ui.label(egui::RichText::new(type_text).size(11.0).strong().color(Color32::WHITE));
                    });

                    // Size column
                    row.col(|ui| {
                        let size_kb = item.size as f32 / 1024.0;
                        ui.label(egui::RichText::new(format!("{:.1} KB", size_kb)).color(Color32::from_rgb(200, 200, 200)));
                    });

                    // Relative path
                    row.col(|ui| {
                        ui.label(
                            egui::RichText::new(&item.rel_path)
                                .size(11.0)
                                .color(Color32::from_rgb(180, 180, 180)),
                        );
                    });
                });
            });

        if let Some(idx) = sample_to_select {
            self.select_sample(idx);
        }
        if let Some(idx) = trigger_drag_row {
            self.select_sample(idx);
            if let Some(path) = self.selected_full_path.clone() {
                self.trigger_drag_export(&path, ui.ctx());
            }
        }
        if let Some(id) = toggle_fav_id {
            self.toggle_favorite(id);
        }
        if trigger_yank_path {
            self.yank_selected_path(ui.ctx());
        }
        if trigger_yank_file {
            self.yank_selected_file(ui.ctx());
        }
        if trigger_reveal {
            self.reveal_selected_file();
        }
        if sort_changed {
            self.run_query();
        }
    }

    fn render_modals(&mut self, ctx: &egui::Context) {
        // Modal: Add Folder Root
        if self.show_add_modal {
            egui::Window::new("Add Sample Library Folder")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Enter directory path containing audio samples, or browse:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_root_path)
                                .hint_text("/path/to/samples or ~/Samples")
                                .desired_width(320.0),
                        );
                        if ui.button("Browse...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.new_root_path = folder.to_string_lossy().to_string();
                                self.add_modal_error = None;
                            }
                        }
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Presets:");
                        if ui.button("demo-samples").clicked() {
                            let p = Path::new("demo-samples");
                            let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                            self.new_root_path = abs.to_string_lossy().to_string();
                            self.add_modal_error = None;
                        }
                        if ui.button("Current Dir").clicked() {
                            if let Ok(cur) = std::env::current_dir() {
                                self.new_root_path = cur.to_string_lossy().to_string();
                                self.add_modal_error = None;
                            }
                        }
                    });

                    if let Some(err) = &self.add_modal_error {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("Error: {err}"))
                                .color(Color32::from_rgb(255, 100, 100))
                                .strong(),
                        );
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Confirm & Index").clicked() {
                            let input = self.new_root_path.trim();
                            if input.is_empty() {
                                self.add_modal_error = Some("Please enter or browse for a directory path".to_string());
                            } else {
                                let expanded = expand_tilde(input);
                                if !expanded.exists() {
                                    self.add_modal_error = Some(format!("Directory does not exist: {}", expanded.display()));
                                } else if !expanded.is_dir() {
                                    self.add_modal_error = Some(format!("Path is not a directory: {}", expanded.display()));
                                } else {
                                    match expanded.canonicalize() {
                                        Ok(canonical) => {
                                            let s = canonical.to_string_lossy().to_string();
                                            if let Err(e) = self.db.conn.execute(
                                                "INSERT INTO roots(path) VALUES (?1) ON CONFLICT(path) DO UPDATE SET enabled=1",
                                                [&s],
                                            ) {
                                                self.add_modal_error = Some(format!("DB error adding root: {e}"));
                                            } else {
                                                let root_id: Result<i64, _> = self.db.conn.query_row(
                                                    "SELECT id FROM roots WHERE path = ?1",
                                                    [&s],
                                                    |r| r.get(0),
                                                );
                                                match root_id {
                                                    Ok(rid) => {
                                                        self.load_metadata();
                                                        self.start_library_scan(rid);
                                                        self.new_root_path.clear();
                                                        self.add_modal_error = None;
                                                        self.show_add_modal = false;
                                                    }
                                                    Err(e) => {
                                                        self.add_modal_error = Some(format!("Failed to retrieve root id: {e}"));
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            self.add_modal_error = Some(format!("Canonicalization failed: {e}"));
                                        }
                                    }
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.new_root_path.clear();
                            self.add_modal_error = None;
                            self.show_add_modal = false;
                        }
                    });
                });
        }

        // Modal: ADSR Importer
        if self.show_adsr_modal {
            egui::Window::new("Import Existing ADSR Database")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Path to ADSR database (e.g. adsr_1_7.db3):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.adsr_db_path)
                            .hint_text("/path/to/adsr_1_7.db3")
                            .desired_width(400.0),
                    );
                    ui.checkbox(&mut self.adsr_dry_run, "Dry run (inspect and verify schema only)");

                    if let Some(err) = &self.adsr_error {
                        ui.label(egui::RichText::new(format!("Error: {err}")).color(Color32::WHITE));
                    }

                    if let Some(rep) = &self.adsr_report {
                        ui.separator();
                        ui.label(egui::RichText::new("ADSR Import Completed:").strong());
                        ui.label(format!("Tables discovered: {}", rep.tables_seen.len()));
                        ui.label(format!("Tags seen: {}", rep.tags_seen));
                        ui.label(format!("Links seen / applied: {} / {}", rep.links_seen, rep.links_applied));
                        ui.label(format!("Favorites preserved: {}", rep.favorites_applied));
                        if !rep.warnings.is_empty() {
                            ui.label(egui::RichText::new(format!("Warnings: {}", rep.warnings.len())).color(Color32::from_rgb(255, 200, 100)));
                        }
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Run Import").clicked() {
                            let p = expand_tilde(self.adsr_db_path.trim());
                            if p.exists() {
                                match scout_import_adsr::import(&p, &self.db_path, &[], self.adsr_dry_run) {
                                    Ok(report) => {
                                        self.adsr_report = Some(report);
                                        self.adsr_error = None;
                                        if !self.adsr_dry_run {
                                            let _ = scout_scanner::tag_all_files(&mut self.db.conn);
                                            self.load_metadata();
                                            self.run_query();
                                        }
                                    }
                                    Err(e) => {
                                        self.adsr_error = Some(format!("{e}"));
                                    }
                                }
                            } else {
                                self.adsr_error = Some(format!("ADSR database file not found at: {}", p.display()));
                            }
                        }
                        if ui.button("Close").clicked() {
                            self.show_adsr_modal = false;
                        }
                    });
                });
        }

        // Modal: Vim Keybindings Help
        if self.show_vim_help {
            egui::Window::new("Vim Keybindings Cheat Sheet")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading("Scout Vim Navigation");
                    ui.separator();
                    ui.label(egui::RichText::new("/            - Focus Search Box").strong());
                    ui.label(egui::RichText::new("<Tab> / <Esc> - Switch focus to Sample List").strong());
                    ui.label(egui::RichText::new("j / <Down>   - Move Down one sample").strong());
                    ui.label(egui::RichText::new("k / <Up>     - Move Up one sample").strong());
                    ui.label(egui::RichText::new("h            - Toggle Favorite [*]").strong());
                    ui.label(egui::RichText::new("l / <Space>  - Play / Pause audition").strong());
                    ui.label(egui::RichText::new("gg           - Jump to First sample").strong());
                    ui.label(egui::RichText::new("G            - Jump to Last sample").strong());
                    ui.label(egui::RichText::new("yp           - Yank Path (copies absolute path)").strong());
                    ui.label(egui::RichText::new("yy           - Yank File (copies file for Finder/DAW)").strong());
                    ui.separator();
                    if ui.button("Close (Esc)").clicked() {
                        self.show_vim_help = false;
                    }
                });
        }
    }
}

fn apply_high_contrast_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::WHITE);

    // Deep high-contrast backgrounds
    visuals.window_fill = Color32::from_rgb(18, 18, 18);
    visuals.panel_fill = Color32::from_rgb(14, 14, 14);
    visuals.faint_bg_color = Color32::from_rgb(24, 24, 24);
    visuals.extreme_bg_color = Color32::from_rgb(8, 8, 8);

    // High-contrast window stroke
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(120, 120, 120));

    // Inactive buttons and widgets
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 26, 26);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(80, 80, 80));
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(230, 230, 230));

    // Hovered widgets
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 45, 45);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.5, Color32::from_rgb(200, 200, 200));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);

    // Active widgets
    visuals.widgets.active.bg_fill = Color32::from_rgb(70, 70, 70);
    visuals.widgets.active.bg_stroke = Stroke::new(2.0, Color32::WHITE);
    visuals.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);

    // Noninteractive labels and frames
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(14, 14, 14);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(70, 70, 70));
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(220, 220, 220));

    // Selection highlight
    visuals.selection.bg_fill = Color32::from_rgb(230, 230, 230);
    visuals.selection.stroke = Stroke::new(1.0, Color32::BLACK);

    ctx.set_visuals(visuals);
}

impl eframe::App for SampleManagerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        apply_high_contrast_theme(&ctx);
        self.poll_scanner();
        self.handle_input(&ctx);

        // macOS-style Top Menu Bar
        egui::Panel::top("macos_menu_bar").show(ui, |ui| {
            self.render_menu_bar(&ctx, ui);
        });

        // Top Search and Filter Bar
        egui::Panel::top("top_panel").show(ui, |ui| {
            self.render_top_bar(ui);
        });

        // Bottom Waveform and Transport Scrubber
        egui::Panel::bottom("bottom_waveform_panel").exact_size(152.0).show(ui, |ui| {
            self.render_bottom_panel(ui);
        });

        // Left Sidebar: Libraries and Tag Taxonomy (with unique ScrollArea IDs)
        egui::Panel::left("left_sidebar")
            .resizable(true)
            .default_size(280.0)
            .min_size(220.0)
            .max_size(480.0)
            .show(ui, |ui| {
                self.render_left_sidebar(ui);
            });

        // Center Virtualized Sample Grid Table
        egui::CentralPanel::default().show(ui, |ui| {
            self.render_central_table(ui);
        });

        self.render_modals(&ctx);
    }
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db_path = PathBuf::from("scout.db");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SCOUT — Sample Library Manager")
            .with_inner_size([1180.0, 750.0])
            .with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "scout-gui",
        native_options,
        Box::new(move |cc| {
            let handle = cc.window_handle().ok().map(|h| StoredWindowHandle(h.as_raw()));
            Ok(Box::new(SampleManagerApp::new(db_path, handle)))
        }),
    )
}
