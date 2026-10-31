# Alternatives — tested, not assumed

Fill every cell from an actual run. "No cloud dependency" is a hard line:
any candidate requiring a cloud catalog for core features is disqualified
even if it is otherwise great.

## Environment record

| Field | Value |
| --- | --- |
| ADSR version / build studied |  |
| Installer platform + architecture |  |
| OS hosts available (mac / win / linux-arm) |  |
| Can the studied build run here? If not, which host was used? |  |
| License terms re: static analysis (read them; note what is allowed) |  |

## Candidates under test

| Candidate (exact version) | License / price | Local-first? | Similarity search? | Auto-tagging source | Filesystem browser? | Drag-out to DAW? | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Sononym (trial x.y) |  |  | yes (verify on your corpus) |  | no (verify) |  | strongest similarity lead; check how it treats library roots |
| <free/OSS or DAW-native browser> |  |  |  |  |  |  | e.g. DAW-native mediabay-style browser |
| <third option, only if quick> |  |  |  |  |  |  |  |

## Measured runs

One run per candidate against the SAME representative library:

| Candidate | Library GB | File count | Cold start to usable | Full scan time | Search latency (est. ms) | Notes |
| --- | --- | --- | --- | --- | --- | --- |
|  |  |  |  |  |  |  |

## Build-vs-buy verdict

Decision: BUILD / BUY / HYBRID
Rationale in 5-10 lines. If a candidate genuinely meets the requirements
(offline, indexed library + free filesystem view, drag-out, tags with
provenance, 1M-file scale), stop and report that instead of building.
