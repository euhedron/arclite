---
id: schemarequested-runs-are-not-revalidated-against
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/synth.rs:1057-1074,1191-1218,1487-1521
- **reason:** Schema-requested runs are not revalidated against their verb item schema: gating checks only for a top-level array, while missing or malformed envelopes can still succeed through prose or raw-JSON fallbacks.
- **rule:** read-structured-data-not-reparsed-prose

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: `synth::run` now recursively validates structured payloads against the verb schema before gating, logging, output, or later promotion can treat them as successful.
