---
id: changing-the-ruleset-invalidates-an-inflight
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/commands/tui.rs:409-416, 439-486, 1300-1314
- **reason:** Changing the ruleset invalidates an in-flight model fetch without resetting `ModelsState::Fetching`, leaving later model-cycle requests permanently waiting.
- **rule:** tolerate-stale-async-results

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `reshape_launch` now resets `ModelsState::Fetching` to `Unfetched` whenever the launch generation changes.
