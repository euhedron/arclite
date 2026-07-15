---
id: multiple-model-identities-are-flattened-into-a
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/ai.rs:191-205; src/synth.rs:1685-1691
- **reason:** Multiple model identities are flattened into a display string with ` + ` and later reparsed by splitting that prose instead of being retained as a structured collection.
- **rule:** read-structured-data-not-reparsed-prose

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: `Usage::models` now retains identities as a vector, and `sum_usage` merges that structured collection before deriving the display string.
