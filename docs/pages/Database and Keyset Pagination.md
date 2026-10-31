---
Type: Persistence
Tags: [sqlite, database, search, fts5, pagination, indexes]
Assoc: "[[INDEX]]", "[[Virtual Scrolling and Infinite Scroll]]", "[[Performance and Benchmarks]]"
Reference: "crates/search/src/lib.rs", "crates/db/src/lib.rs"
Created: 2026-10-31
---

# Database and Keyset Pagination

LocSamp relies on an embedded SQLite database managed via `rusqlite` bundled with WAL (Write-Ahead Logging) and custom collation functions.

---

## Schema Architecture

```mermaid
erDiagram
    ROOTS ||--o{ FILES : contains
    FILES ||--o{ FILE_TAGS : has
    TAGS ||--o{ FILE_TAGS : categorizes
    FILES ||--o{ FAVORITES : marked_as
    MUSICAL_KEYS ||--o{ FILES : key_of
    FILES ||--|| FILES_FTS : full_text_index

    ROOTS {
        integer id PK
        text path UK
        integer enabled
    }
    FILES {
        integer id PK
        integer root_id FK
        text rel_path
        text name
        text ext
        integer duration_ms
        real bpm
        integer key_id FK
        integer is_loop
        integer size
        integer mtime
        integer missing_since
    }
    TAGS {
        integer id PK
        text name UK
        text kind
        text source
    }
    FILE_TAGS {
        integer file_id FK
        integer tag_id FK
        text state
        real confidence
    }
    FAVORITES {
        integer file_id PK
        integer added_at
    }
```

---

## Key Optimization Techniques

### 1. Natural Sort Collation (`NATURALNOCASE`)
Standard SQL sorting sorts alphanumeric strings lexicographically (`Kick 1`, `Kick 10`, `Kick 2`). LocSamp registers a custom SQLite collation that chunks digits and strings, providing natural sorting directly in SQL:
```rust
db.create_collation("NATURALNOCASE", natural_sort_cmp)?;
```
Sorting produces `Kick 1`, `Kick 2`, `Kick 10`.

### 2. Window Function Total Counting
Queries execute with `LIMIT ? OFFSET ?`. To avoid running two separate queries (`SELECT ...` and `SELECT count(*)`), LocSamp uses the SQLite window function:
```sql
count(*) OVER () AS total
```
This computes the total number of matching rows in the query dataset in a single query pass.

### 3. FTS5 Quoted Prefix Queries
Search input is tokenized and quoted to prevent syntax errors:
```rust
pub fn fts_expr(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" ")) }
}
```
Queries like `open hat` become `"open"* "hat"*` against `files_fts`.
