#!/usr/bin/env bash
# inspect-adsr-db.sh <path-to-COPY-of-adsr-db> [out-dir]
# Read-only by construction: copy first, then open the copy with
# sqlite3 -readonly. The original is never opened by sqlite at all.
set -euo pipefail

SRC="$1"
OUTDIR="adsr-db-report"
if [ $# -ge 2 ]; then OUTDIR="$2"; fi

if [ ! -f "$SRC" ]; then
  echo "error: no such file: $SRC" >&2
  echo "hint: pass a COPY of the database, never the live file" >&2
  exit 1
fi

mkdir -p "$OUTDIR"
COPY="$OUTDIR/working-copy.sqlite"
cp "$SRC" "$COPY"
# copy WAL/SHM siblings if present so the copy is self-consistent
[ -f "$SRC-wal" ] && cp "$SRC-wal" "$COPY-wal" || true
[ -f "$SRC-shm" ] && cp "$SRC-shm" "$COPY-shm" || true

echo "== engine / user_version ==" | tee "$OUTDIR/00-meta.txt"
sqlite3 -readonly "$COPY" "SELECT sqlite_version(); PRAGMA user_version;"   | tee -a "$OUTDIR/00-meta.txt"

echo "== tables + row counts ==" | tee "$OUTDIR/10-tables.txt"
sqlite3 -readonly "$COPY" ".tables" | tr ' ' '
' | sed '/^$/d' | sort   > "$OUTDIR/90-table-list.txt"
while read -r T; do
  N=$(sqlite3 -readonly "$COPY" "SELECT count(*) FROM "$T";")
  echo "$T  rows=$N" | tee -a "$OUTDIR/10-tables.txt"
done < "$OUTDIR/90-table-list.txt"

sqlite3 -readonly "$COPY" ".schema" > "$OUTDIR/20-schema.sql"
sqlite3 -readonly "$COPY"   "SELECT name, tbl_name FROM sqlite_master WHERE type='index' ORDER BY tbl_name;"   > "$OUTDIR/30-indexes.txt"
sqlite3 -readonly "$COPY"   "SELECT name, tbl_name, sql FROM sqlite_master WHERE type='trigger';"   > "$OUTDIR/40-triggers.txt"

# Query-plan snapshots for the workflows that matter. Adapt table/column
# names to whatever step 10 actually reports.
{
  echo '-- example: name search (adjust to actual schema)'
  echo "-- EXPLAIN QUERY PLAN SELECT * FROM <files> WHERE <name> LIKE '%kick%' LIMIT 50;"
  echo '-- example: tag filter join'
  echo '-- EXPLAIN QUERY PLAN SELECT ...'
} > "$OUTDIR/50-query-plans.txt"

echo "done. report in $OUTDIR — summarize into docs/re/adsr-db-notes.md"
