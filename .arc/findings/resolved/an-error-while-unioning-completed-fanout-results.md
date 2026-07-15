---
id: an-error-while-unioning-completed-fanout-results
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/synth.rs:1451-1458, 1473, 1512-1528
- **reason:** An error while unioning completed fan-out results propagates before logging and drops the already-summed usage of every completed call.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `multi_synthesize` now catches combination failures and returns an errored synthesis carrying the fan-out's summed usage for logging.
