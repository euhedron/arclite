---
id: render-projections-call-displaypath-which-probes
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/lib.rs:184-195; src/commands/doctor.rs:323-342
- **reason:** Render projections call `display_path`, which probes the home-directory environment while formatting instead of reading a precomputed value from state.
- **rule:** render-is-a-pure-function-of-state

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: The home-directory prefix is now stored in `DISPLAY_HOME` and warmed at startup before render projections call `display_path`.
