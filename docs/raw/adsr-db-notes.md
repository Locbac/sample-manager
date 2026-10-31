# ADSR DB notes — from a read-only copy

Non-negotiables
  - Never open the user's original DB. Copy first, inspect the copy.
  - Open read-only (see scripts/inspect-adsr-db.sh). No VACUUM, no writes,
    no migrations, no optimizing "just this one index".
  - This document feeds an OPTIONAL importer for the user's own data. The
    new app never depends on ADSR's live database or schema.

## Schema capture

Run scripts/inspect-adsr-db.sh and summarize below.

| Area | Finding |
| --- | --- |
| DB engine + user_version |  |
| Tables (name, rough row counts) |  |
| Files table: columns that matter |  |
| Tags table + how hierarchy is represented |  |
| file<->tag link table (any source/confidence columns?) |  |
| Favorites / collections / playlists |  |
| Key + BPM enum representation |  |
| Indexes present |  |
| Triggers / FTS tables present |  |

## Enum spot-checks (verify against the UI; prelim mappings are leads only)

| enum value | UI shows | evidence |
| --- | --- | --- |
| key = 0 |  |  |
| key = 1 |  |  |
| bpm bucket mapping |  |  |

## Query plans

Paste EXPLAIN QUERY PLAN for: name search, tag filter, folder listing,
favorites. Note which queries have covering indexes — that reveals which
workflows they optimize, which is useful signal for our own schema.

## Import-relevant observations

- Worth importing: tags (yes), favorites (yes), collections (if
  recoverable), user notes (if any exist).
- Path formats observed (win drive / UNC / posix) and normalization plan ->
- What must remain unknown in V1 ->
