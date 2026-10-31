---
Type: Performance
Tags: [benchmarks, indexing, throughput, latency, optimization]
Assoc: "[[INDEX]]", "[[Database and Keyset Pagination]]", "[[ADSR Reverse Engineering Methodology]]"
Reference: "crates/scanner/src/lib.rs", "docs/benchmarks.md"
Created: 2026-10-31
---

# Performance and Benchmarks

LocSamp is designed around minimal I/O overhead, zero redundant file reads, and vectorized transaction batching.

---

## 150 GB Producer Library Real-World Benchmark

Conducted on a real-world producer library containing ~150 GB of sample packs (`151130364 Sample Packs/` via `du -k`):

| Environment | Storage Type | LocSamp Index Time | ADSR Sample Manager Index Time | Speedup |
| :--- | :--- | :--- | :--- | :--- |
| **macOS (M-Series)** | NVMe SSD | **~30 seconds** | **~60 minutes** | **$120\times$ faster** |
| **Windows 11** | 7200 RPM HDD | **~2 minutes** | **7 to 8 hours** | **$225\times$ faster** |

```text
Full Library Scan (~150 GB / 151M blocks)
─────────────────────────────────────────────────────────────────────────────
LocSamp (SSD)               [  30s ]  █
ADSR Sample Manager (SSD)   [ ~60m ]  ████████████████████
ADSR Sample Manager (HDD)   [~450m ]  ███████████████████████████████████████
─────────────────────────────────────────────────────────────────────────────
Time (shorter is better)
```

---

## Why Is LocSamp So Much Faster?

1. **Header-Only Probe**:
   Instead of decoding entire audio files or parsing full streams into memory during indexing, LocSamp extracts duration, channels, bit depth, and sample rate by parsing container headers via Symphonia/Lofty.

2. **Zero-Lock Transaction Batching**:
   Files are scanned using recursive directory iterators and batched into chunks of 256 records inside SQLite transactions. Writing 100,000 files requires only ~400 disk syncs rather than 100,000 fsync calls.

3. **In-Memory Token Matching**:
   Tagging rules run pure string and word-boundary matching in CPU L1/L2 cache, taking $< 5\ \mu\text{s}$ per file.

4. **Instant Incremental Rescans**:
   On rescans, LocSamp compares `(path, size, mtime)` against the database index. Unmodified files are skipped in microseconds without opening the file.
