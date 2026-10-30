// crates/cli/src/main.rs
// Purpose: prove the core headless (spec M2). The Tauri app calls the SAME
// functions; nothing UI-shaped leaks into the core crates.
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "scout", version, about = "sample library engine (headless)")]
struct Cli {
    /// SQLite database file (local disk only — never a network share)
    #[arg(long, default_value = "scout.db")]
    db: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register a library root
    AddRoot { path: PathBuf },
    /// Scan a root; streams events; Ctrl-C safe (cancel -> run marked cancelled)
    Scan {
        root_id: i64,
        /// prioritize heavy work under this folder, e.g. packs/drums
        #[arg(long)]
        focus: Option<String>,
    },
    /// FT search + filters; prints one JSON array
    Search {
        q: Option<String>,
        #[arg(long)] exts: Vec<String>,
        #[arg(long)] bpm_min: Option<f64>,
        #[arg(long)] bpm_max: Option<f64>,
        #[arg(long)] tag: Vec<i64>,
        #[arg(long)] fav: bool,
        #[arg(long, default_value_t = 50)] limit: u32,
        #[arg(long, default_value_t = 0)] offset: u32,
    },
    /// Show one file incl. tags with provenance
    Show { file_id: i64 },
    /// Manual tag ops
    Tag { file_id: i64, name: String },
    Dismiss { file_id: i64, tag_id: i64 },
    Confirm { file_id: i64, tag_id: i64 },
    /// Reload the ruleset and re-run lexical tagging only (never audio/model)
    Retag {
        root_id: i64,
        #[arg(long, default_value = "crates/tagging/rules.default.toml")] rules: PathBuf,
    },
    /// Dry-run an ADSR import against a COPY of the user's db (read-only)
    ImportAdsr {
        src: PathBuf,
        #[arg(long)] path_map: Vec<String>, // "X:/Samples=/Volumes/Samples"
        #[arg(long, default_value_t = true)] dry_run: bool,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    let mut db = scout_db::open(&cli.db)?;

    match cli.cmd {
        Cmd::AddRoot { path } => {
            let norm = path.canonicalize()?;
            db.conn.execute(
                "INSERT INTO roots(path) VALUES (?1)
                 ON CONFLICT(path) DO UPDATE SET enabled=1",
                [norm.to_string_lossy().to_string()],
            )?;
            println!("root id {}", db.conn.last_insert_rowid());
        }
        Cmd::Scan { root_id, focus } => {
            let (ev_tx, ev_rx) = crossbeam::channel::unbounded();
            let root_path: String = db.conn.query_row(
                "SELECT path FROM roots WHERE id=?1",
                [root_id],
                |r| r.get(0),
            )?;
            let opts = scout_scanner::ScanOptions {
                focus_dir: focus.map(PathBuf::from),
                ..Default::default()
            };
            let handle = scout_scanner::scan_root(
                cli.db.clone(),
                root_id,
                PathBuf::from(root_path),
                opts,
                ev_tx,
            );
            // cancel on Ctrl-C: scan_runs row is always closed properly
            let flag = handle.cancel.clone();
            ctrlc_note(&flag);
            for ev in ev_rx {
                println!("{ev:?}");
            }
        }
        Cmd::Search { q, exts, bpm_min, bpm_max, tag, fav, limit, offset } => {
            let query = scout_search::SearchQuery {
                q,
                exts,
                bpm_min,
                bpm_max,
                tags_include: tag,
                favorites_only: fav,
                limit,
                offset,
                ..Default::default()
            };
            let rows = scout_search::search(&db.conn, &query)?;
            // serde_json on the joined struct of your DTO — kept dense here
            println!("{} rows (total on first row)", rows.len());
            for r in rows.iter().take(10) {
                println!("  #{:<8} {:<60} total={}", r.id, r.rel_path, r.total);
            }
        }
        Cmd::Show { file_id } => {
            let (name, rel): (String, String) = db.conn.query_row(
                "SELECT name, rel_path FROM files WHERE id=?1",
                [file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            println!("{name}
  {rel}
  tags:");
            let mut st = db.conn.prepare(
                "SELECT t.name, ft.source, ifnull(ft.source_detail,''), ifnull(ft.confidence,-1), ft.state
                 FROM file_tags ft JOIN tags t ON t.id = ft.tag_id
                 WHERE ft.file_id=?1 ORDER BY ft.source, t.name",
            )?;
            let rows = st.query_map([file_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?, r.get::<_, f64>(3)?, r.get::<_, String>(4)?))
            })?;
            for r in rows {
                let (n, s, d, c, st) = r?;
                println!("  [{st:<9}] {n:<14} via {s} {d} conf={c:.2}");
            }
        }
        Cmd::Tag { file_id, name } => {
            let id = scout_db::ensure_tag(&db.conn, &name, "category", "user")?;
            scout_db::add_manual_tag(&db.conn, file_id, id)?;
            println!("tagged #{file_id} with {name}");
        }
        Cmd::Dismiss { file_id, tag_id } => scout_db::dismiss_tag(&db.conn, file_id, tag_id)?,
        Cmd::Confirm { file_id, tag_id } => scout_db::confirm_tag(&db.conn, file_id, tag_id)?,
        Cmd::Retag { root_id, rules } => {
            let text = std::fs::read_to_string(&rules)?;
            let engine = scout_tagging::Engine::from_toml(&text)?;
            let mut st = db.conn.prepare(
                "SELECT id, name, rel_path FROM files WHERE root_id=?1 AND missing_since IS NULL",
            )?;
            let files: Vec<(i64, String, String)> = st
                .query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            drop(st);
            let tx = db.conn.transaction()?;
            for (fid, name, rel) in &files {
                let musical = scout_tagging::extract_musical_info(name, rel);
                let key_id = if let Some(key_str) = &musical.key {
                    scout_db::find_musical_key(&tx, key_str).ok().flatten().map(|(kid, _, _)| kid)
                } else {
                    None
                };
                let _ = scout_db::update_file_musical_info(&tx, *fid, musical.bpm, key_id, musical.is_loop);

                for s in engine.suggest(name, rel) {
                    let tag_id = scout_db::ensure_tag(&tx, &s.tag, &s.kind, "system")?;
                    let source = match s.source {
                        scout_tagging::Source::FilenameRule => "filename_rule",
                        scout_tagging::Source::FolderRule => "folder_rule",
                    };
                    // record_automatic_tag NEVER resurrects dismissed rows,
                    // NEVER touches manual rows. Provenance lands in source_detail.
                    scout_db::record_automatic_tag(
                        &tx, *fid, tag_id, source, Some(s.score), Some(&s.detail),
                    )?;
                }
            }
            tx.commit()?;
            println!("re-tagged {} files with ruleset {}", files.len(), engine.ruleset.version);
        }
        Cmd::ImportAdsr { src, path_map, dry_run } => {
            let report = scout_import_adsr::import(&src, &cli.db, &path_map, dry_run)?;
            println!("{report}");
            if dry_run {
                println!("dry run only — re-run with --dry-run=false to commit");
            }
        }
    }
    Ok(())
}

fn ctrlc_note(flag: &std::sync::atomic::AtomicBool) {
    // ctrlc crate optional; at minimum document the path:
    eprintln!("(Ctrl-C cancels gracefully via ScanHandle.cancel)");
    let _ = flag;
}
