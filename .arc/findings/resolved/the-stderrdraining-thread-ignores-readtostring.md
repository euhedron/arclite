---
id: the-stderrdraining-thread-ignores-readtostring
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/ai.rs:687-694
- **reason:** The stderr-draining thread ignores `read_to_string` failures, collapsing an unreadable child error stream into empty or partial stderr.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: The stderr-draining thread now records a visible capture-failure marker when `read_to_string` fails partway.
