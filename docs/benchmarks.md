# Benchmarks (spec 5.3 — measure, do not promise)

## Reference machine

| Field | Value |
| --- | --- |
| CPU / cores / arch |  |
| RAM |  |
| Storage (NVMe/SATA/HDD) |  |
| OS + version |  |
| App build (git sha, profile) |  |

## Libraries under test

| Library | GB | Files | Composition |
| --- | --- | --- | --- |
| synthetic-1m | ~0 (paths only) / +N GB wav if generated | 1,000,000 | generator in spikes/sqlite-bench |
| user-real |  |  | real folder mix: wav/aif/mp3/flac |

## Workloads + results

| Workload | Target | Run 1 | Run 2 | Run 3 | p50 | p95 | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Cold scan (stage 1 walk+upsert) | report files/s |  |  |  |  |  | storage-bound: REPORT, not promised |
| Header probe rate |  |  |  |  |  |  |  |
| Warm open to browsable list | interactive immediately |  |  |  |  |  | no wait-for-scan allowed |
| Query: fts prefix | p95 < 100 ms |  |  |  |  |  |  |
| Query: tag filter join | p95 < 100 ms |  |  |  |  |  |  |
| Query: bpm range + sort | p95 < 100 ms |  |  |  |  |  |  |
| Query: substring LIKE | informational |  |  |  |  |  | if bad => trigram FTS ticket |
| Scroll 500k result window | 60 fps |  |  |  |  |  | virtualization check |
| Preview start (SSD/NVMe) | < 300 ms |  |  |  |  |  |  |
| Scan CPU/I/O background | stays interactive |  |  |  |  |  | measure with top/iostat |
| RSS after 1h session |  |  |  |  |  |  |  |

## Reproduce

  cargo run --release -p scout-cli -- scan 1
  cargo run --release -p scout-cli -- search kick --limit 50
  cargo run --release --manifest-path spikes/sqlite-bench/Cargo.toml

Keep raw command output in benchmarks/raw/<date>-<machine>.txt
