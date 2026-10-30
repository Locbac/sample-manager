// spikes/sqlite-bench/src/main.rs
// Cargo deps: rusqlite = { features = ["bundled"] }
// Pass criteria (spec): common search/filter p95 < 100 ms warm @ 1M rows.
// This is the "measure, don't promise" harness from spec 5.3.
use rusqlite::{params, Connection};
use std::time::Instant;

const N: usize = 1_000_000;
const SELECTORS: [&str; 8] = ["kick", "snare", "hat", "clap", "loop", "bass", "fx", "pad"];

fn quantiles(mut v: Vec<u128>) -> (u128, u128, u128, u128) {
    v.sort_unstable();
    let pick = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
    (
        v.iter().sum::<u128>() / v.len() as u128,
        pick(0.50),
        pick(0.95),
        pick(0.99),
    )
}

fn time_query<F: FnMut() -> rusqlite::Result<usize>>(label: &str, mut f: F, iters: usize) {
    f().unwrap(); // warm
    let mut us = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        let rows = f().unwrap();
        assert!(rows > 0);
        us.push(t.elapsed().as_micros());
    }
    let (avg, p50, p95, p99) = quantiles(us);
    println!("{label:<34} avg={avg:<7}us p50={p50:<7}us p95={p95:<7}us p99={p99}us");
}

fn main() -> rusqlite::Result<()> {
    let conn = Connection::open_in_memory()?; // also run against a file-backed DB for realism
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(
        "CREATE TABLE files(
           id INTEGER PRIMARY KEY, root INT, rel TEXT NOT NULL, name TEXT NOT NULL,
           ext TEXT NOT NULL, dur_ms INT, bpm REAL, size INT, fav INT DEFAULT 0);
         CREATE INDEX ix_files_rel ON files(root, rel);
         CREATE INDEX ix_files_bpm ON files(bpm) WHERE bpm IS NOT NULL;
         CREATE VIRTUAL TABLE files_fts USING fts5(name, rel, content='files', content_rowid='id');
         CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE file_tags(file_id INT, tag_id INT, UNIQUE(file_id, tag_id));
         CREATE INDEX ix_ft_tag ON file_tags(tag_id, file_id);",
    )?;

    // --- ingest --------------------------------------------------------------
    let t0 = Instant::now();
    conn.execute("INSERT INTO tags VALUES (1,'kick'),(2,'snare'),(3,'loop')", [])?;
    let mut id = 0usize;
    while id < N {
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO files(root,rel,name,ext,dur_ms,bpm,size,fav)
                 VALUES (1,?1,?2,?3,?4,?5,?6,?7)",
            )?;
            let mut fts =
                tx.prepare("INSERT INTO files_fts(rowid,name,rel) VALUES (?1,?2,?3)")?;
            let mut ft = tx.prepare("INSERT OR IGNORE INTO file_tags VALUES (?1,?2)")?;
            for _ in 0..10_000 {
                id += 1;
                let sel = SELECTORS[id % SELECTORS.len()];
                let pack = id / 10_000;
                let name = format!("{sel}_{}.wav", id % 1000);
                let rel = format!("packs/pack-{pack:03}/sub-{}/{}", id % 40, name);
                let bpm = if sel == "loop" { Some(60.0 + (id % 130) as f64) } else { None };
                ins.execute(params![rel, name, "wav", 1200i64, bpm, 480_000i64, (id % 97 == 0) as i64])?;
                let rowid = tx.last_insert_rowid();
                fts.execute(params![rowid, name, rel])?;
                ft.execute(params![rowid, ((id % 3) + 1) as i64])?;
            }
        }
        tx.commit()?;
        if id % 100_000 == 0 {
            println!("ingested {id}/{N}  ({:?} total)", t0.elapsed());
        }
    }
    println!("INGEST 1M rows: {:?}", t0.elapsed());

    // --- workload ------------------------------------------------------------
    let it = 60;
    time_query("fts prefix: kick*", || {
        conn.prepare("SELECT id FROM files_fts WHERE files_fts MATCH 'name:kick*' LIMIT 50")?
            .query_map([], |r| r.get::<_, i64>(0))
            .map(|it| it.count())
    }, it);
    time_query("substring LIKE '%kik_7%'", || {
        conn.prepare("SELECT id FROM files WHERE name LIKE '%kik_7%' LIMIT 50")?
            .query_map([], |r| r.get::<_, i64>(0))
            .map(|it| it.count())
    }, it);
    time_query("bpm 118..126 + sort + limit", || {
        conn.prepare(
            "SELECT id FROM files WHERE bpm BETWEEN 118 AND 126 ORDER BY bpm LIMIT 50",
        )?
        .query_map([], |r| r.get::<_, i64>(0))
        .map(|it| it.count())
    }, it);
    time_query("tag join + order by name", || {
        conn.prepare(
            "SELECT f.id FROM files f JOIN file_tags t ON t.file_id=f.id
             WHERE t.tag_id=1 ORDER BY f.name LIMIT 50",
        )?
        .query_map([], |r| r.get::<_, i64>(0))
        .map(|it| it.count())
    }, it);
    time_query("fav + fts combined", || {
        conn.prepare(
            "SELECT f.id FROM files f JOIN files_fts x ON x.rowid=f.id
             WHERE files_fts MATCH 'name:kick*' AND f.fav=1 LIMIT 50",
        )?
        .query_map([], |r| r.get::<_, i64>(0))
        .map(|it| it.count())
    }, it);

    // If substring LIKE p95 is bad (it will be, at 1M rows), that is the
    // evidence for adding a trigram FTS table for name — record in
    // docs/benchmarks.md and only then add the index.
    Ok(())
}
