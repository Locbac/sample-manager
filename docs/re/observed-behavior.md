# Observed behavior — what actually drives automatic tags

Rules of engagement
  - Test corpus: 100-300 self-made or freely-licensed sounds ONLY, with
    hand-recorded ground truth in docs/re/test-corpus.csv.
  - Work on copies. Note ADSR version per trial. Clear/namespace caches
    between trials so one scan cannot contaminate the next.
  - Confidence per finding: HIGH (reproduced twice, clean isolation),
    MED (single run or partial isolation), LOW (inference).

## Experiment matrix (one variable per experiment)

| ID | Setup | Distinguishes | Decision rule |
| --- | --- | --- | --- |
| E1 | same audio bytes, 3 filenames (kick_01 / snare_top / zzz) | audio vs filename | tags follow the name => lexical path primary |
| E2 | 3 different audios, same neutral name (a1.wav) | audio vs filename | tags differ per audio => content analysis exists |
| E3 | same audio, 3 parent folders (Kicks/ Synths/ Misc/) | folder context | folder terms appear as weaker suggestions |
| E4 | misleading name: a snare named kick_01.wav | word-only tagging | tag with zero audio evidence => pure lexical path |
| E5 | same audio, with vs without embedded metadata | metadata | metadata-only tag appears after fresh rescan |
| E6 | rename file, rescan, diff tags | recompute vs retention | recomputed vs sticky across rename |
| E7 | duplicate bytes at a second path | content dedup | shared identity vs per-path reanalysis |
| E8 | fully offline host after first setup | local vs service | any degradation when network is cut |

## Trial log (append one row per run)

| trial | date | adsr_version | experiment | input | tags observed | db delta | duration_ms | confidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| T001 |  |  | E1 |  |  |  |  |  |

## Questions this must answer (fill with evidence pointers)

1. Tag sources: filename / folders / embedded / audio / service? ->
2. Multiple audio-derived labels per file? ->
3. User edits on rescan: kept or overwritten? ->
4. Generated vs manual tags distinguishable in the DB? ->
5. Tags per-path or per-duplicate-content? ->
6. Which generic categories are worth reproducing? ->
7. Ambiguous low-confidence behavior (tom_1.wav sounding kick-like)? ->

## Unanswered questions that would change the implementation

| Question | Why it matters | Planned resolution |
| --- | --- | --- |
|  |  |  |
