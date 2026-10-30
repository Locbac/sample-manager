//! crates/scanner — staged indexing (spec 5.1)
//! Stage 1: walk + stat + upsert  (no file opened — instant browsability)
//! Stage 2: header probe          (codec params only, NOT full decode)
//! Stage 3+: queued in 'jobs'     (hash / peaks / classify / keybpm)
//! Everything is cancellable, pausable, resumable, and event-driven.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam::channel::Sender;
use walkdir::WalkDir;

pub const AUDIO_EXTS: &[&str] = &[
    "wav", "aif", "aiff", "flac", "mp3", "m4a", "aac", "ogg", "opus",
];

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Started { root_id: i64, run_id: i64 },
    Progress { seen: u64, new: u64, changed: u64 },
    Row { file_id: i64, rel_path: String, is_new: bool }, // UI can insert rows live
    RootDone { root_id: i64, run_id: i64, missing: u64 },
    Paused,
    Cancelled,
    Failed { message: String },
}

#[derive(Clone)]
pub struct ScanOptions {
    pub include_hidden: bool,
    pub focus_dir: Option<PathBuf>, // currently browsed dir => its jobs get priority 100
    pub batch_size: usize,          // rows per transaction (256-1024 sane)
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { include_hidden: false, focus_dir: None, batch_size: 512 }
    }
}

pub struct ScanHandle {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Runs on a worker thread. 'db_path' — each worker opens its own connection.
pub fn scan_root(
    db_path: PathBuf,
    root_id: i64,
    root_path: PathBuf,
    opts: ScanOptions,
    tx: Sender<ScanEvent>,
) -> ScanHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let (c_worker, p_worker) = (cancel.clone(), pause.clone());

    let _handle = std::thread::spawn(move || {
        if let Err(e) = run(&db_path, root_id, &root_path, &opts, &tx, &c_worker, &p_worker) {
            let _ = tx.send(ScanEvent::Failed { message: e.to_string() });
        }
    });
    ScanHandle { cancel, pause }
}

fn run(
    db_path: &Path,
    root_id: i64,
    root_path: &Path,
    opts: &ScanOptions,
    tx: &Sender<ScanEvent>,
    cancel: &Arc<AtomicBool>,
    pause: &Arc<AtomicBool>,
) -> Result<()> {
    let mut db = scout_db::open(db_path)?;
    let c = &mut db.conn;

    c.execute("INSERT INTO scan_runs(root_id) VALUES (?1)", [root_id])?;
    let run_id: i64 = c.last_insert_rowid();
    let _ = tx.send(ScanEvent::Started { root_id, run_id });

    let mut seen = 0u64;
    let mut new = 0u64;
    let changed = 0u64;
    let mut last_emit = Instant::now();

    let walker = WalkDir::new(root_path).follow_links(false).into_iter();
    let entries = walker.filter_entry(|e| {
        opts.include_hidden
            || e.depth() == 0
            || !e.file_name().to_string_lossy().starts_with('.')
    });

    // Batched write transaction: row upserts are ~free, one commit per batch.
    let mut pending: Vec<(String, String, String, i64, i64)> = Vec::with_capacity(opts.batch_size);
    let flush = |c: &mut rusqlite::Connection,
                 pending: &mut Vec<(String, String, String, i64, i64)>,
                 out: &mut Vec<(i64, bool, String)>|
     -> Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        let txb = c.transaction()?;
        let engine = scout_tagging::default_engine();
        for (rel, name, ext, size, mtime) in pending.drain(..) {
            let tokens = scout_tagging::tokenize(&name).join(" ");
            let (id, was_changed) = scout_db::upsert_seen_file(
                &txb, root_id, &rel, &name, &tokens, &ext, size, mtime, run_id,
            )?;

            // Auto-assign tags based on file name & parent folder hierarchy
            for s in engine.suggest(&name, &rel) {
                if let Ok(tag_id) = scout_db::ensure_tag(&txb, &s.tag, "category", "system") {
                    let source = match s.source {
                        scout_tagging::Source::FilenameRule => "filename_rule",
                        scout_tagging::Source::FolderRule => "folder_rule",
                    };
                    let _ = scout_db::record_automatic_tag(
                        &txb, id, tag_id, source, Some(s.score), Some(&s.detail),
                    );
                }
            }

            out.push((id, was_changed, rel));
        }
        txb.commit()?;
        Ok(())
    };

    let mut upserted: Vec<(i64, bool, String)> = Vec::with_capacity(opts.batch_size);

    for entry in entries {
        // --- cooperative pause/cancel: the UI stays alive during 1M walks --
        while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
        }
        if cancel.load(Ordering::Relaxed) {
            finish_run(c, run_id, seen, new, changed, 0, "cancelled")?;
            let _ = tx.send(ScanEvent::Cancelled);
            return Ok(());
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // permission errors must not kill the scan
        };
        if !entry.file_type().is_file() || !is_audio(entry.path()) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let rel = entry
            .path()
            .strip_prefix(root_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('/', "/"); // normalized separators
        let name = entry.file_name().to_string_lossy().to_string();
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        seen += 1;
        pending.push((rel, name, ext, meta.len() as i64, mtime));
        if pending.len() >= opts.batch_size {
            flush(c, &mut pending, &mut upserted)?;
        }
        if last_emit.elapsed() > Duration::from_millis(150) {
            last_emit = Instant::now();
            let _ = tx.send(ScanEvent::Progress { seen, new, changed });
        }
    }
    flush(c, &mut pending, &mut upserted)?;

    // Stage-2 + stage-3 work, prioritized by what the user is browsing.
    let txb = c.transaction()?;
    for (id, was_changed, rel) in &upserted {
        let prio = if opts
            .focus_dir
            .as_ref()
            .map(|f| opts
                .focus_dir
                .as_ref()
                .map(|fd| rel.starts_with(&fd.to_string_lossy().to_string()))
                .unwrap_or(false) || f.as_os_str().is_empty())
            .unwrap_or(false)
        {
            100
        } else {
            10
        };
        if *was_changed {
            txb.execute(
                "INSERT INTO jobs(kind, file_id, priority) VALUES ('header', ?1, ?2)",
                rusqlite::params![id, prio],
            )?;
            let _ = tx.send(ScanEvent::Row { file_id: *id, rel_path: rel.clone(), is_new: true });
            new += 1;
        }
        let _ = was_changed;
    }
    txb.commit().map(|_| ()).or_else(|e| Err(e))?;

    // Stage 2 inline (fast): probe headers for the just-enqueued focus files.
    // (Full decode stays out. Hash/peaks/classify wait for the job worker.)
    let mut stmt = c.prepare(
        "SELECT f.id, f.rel_path FROM files f
         JOIN jobs j ON j.file_id = f.id AND j.kind='header' AND j.status='queued'
         WHERE f.root_id=?1 ORDER BY j.priority DESC LIMIT 500",
    )?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    for (fid, rel) in rows {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(hdr) = probe_header(&root_path.join(&rel)) {
            c.execute(
                "UPDATE files SET duration_ms=?2, sample_rate=?3, channels=?4 WHERE id=?1",
                rusqlite::params![fid, hdr.duration_ms, hdr.sample_rate, hdr.channels],
            )?;
        }
        c.execute(
            "UPDATE jobs SET status='done', updated_at=CURRENT_TIMESTAMP
             WHERE file_id=?1 AND kind='header' AND status='queued'",
            [fid],
        )?;
        // heavy jobs follow the header, now keyed for content-dedup later
        c.execute(
            "INSERT INTO jobs(kind, file_id, priority) VALUES ('hash', ?1, 10)",
            [fid],
        )?;
    }

    let missing = scout_db::sweep_missing(c, root_id, run_id)? as u64;
    finish_run(c, run_id, seen, new, changed, missing, "done")?;
    c.execute(
        "UPDATE roots SET last_scan_finished_at=CURRENT_TIMESTAMP,
                file_count=(SELECT count(*) FROM files WHERE root_id=?1 AND missing_since IS NULL)
         WHERE id=?1",
        [root_id],
    )?;
    let _ = tx.send(ScanEvent::RootDone { root_id, run_id, missing });
    Ok(())
}

/// Batch-tag all existing files in the database using the lexical rules engine.
pub fn tag_all_files(c: &mut rusqlite::Connection) -> Result<usize> {
    let engine = scout_tagging::default_engine();
    let mut stmt = c.prepare("SELECT id, name, rel_path FROM files WHERE missing_since IS NULL")?;
    let files: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    let mut tagged = 0;
    let txb = c.transaction()?;
    for (fid, name, rel) in &files {
        for s in engine.suggest(name, rel) {
            if let Ok(tag_id) = scout_db::ensure_tag(&txb, &s.tag, "category", "system") {
                let source = match s.source {
                    scout_tagging::Source::FilenameRule => "filename_rule",
                    scout_tagging::Source::FolderRule => "folder_rule",
                };
                let _ = scout_db::record_automatic_tag(
                    &txb, *fid, tag_id, source, Some(s.score), Some(&s.detail),
                );
                tagged += 1;
            }
        }
    }
    txb.commit()?;
    Ok(tagged)
}

fn finish_run(
    c: &rusqlite::Connection,
    run_id: i64,
    seen: u64,
    new: u64,
    changed: u64,
    missing: u64,
    status: &str,
) -> Result<()> {
    c.execute(
        "UPDATE scan_runs SET finished_at=CURRENT_TIMESTAMP, files_seen=?2,
               files_new=?3, files_changed=?4, files_missing=?5, status=?6
         WHERE id=?1",
        rusqlite::params![run_id, seen, new, changed, missing, status],
    )?;
    Ok(())
}

pub struct HeaderInfo {
    pub duration_ms: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
}

/// Codec-params-only probe. VBR durations are estimates — that is fine for
/// stage 2; ground truth arrives when the content hash/decode job runs.
pub fn probe_header(path: &Path) -> Option<HeaderInfo> {
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::probe::Hint;
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &Default::default())
        .ok()?;
    let track = probed.format.default_track()?;
    let p = &track.codec_params;
    let duration_ms = match (p.n_frames, p.time_base) {
        (Some(n), Some(tb)) => {
            Some((n as f64 * tb.numer as f64 * 1000.0 / tb.denom as f64) as i64)
        }
        _ => None,
    };
    Some(HeaderInfo {
        duration_ms,
        sample_rate: p.sample_rate.map(|v| v as i64),
        channels: p.channels.map(|c| c.count() as i64),
    })
}


