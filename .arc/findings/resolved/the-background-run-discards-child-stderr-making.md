---
id: the-background-run-discards-child-stderr-making
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/commands/tui.rs:528-539
- **reason:** The background run discards child stderr, making warnings about failed logging, result storage, and status-marker writes invisible to the TUI user.
- **rule:** best-effort-side-effects

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: Background runs now pipe and drain stderr, then surface failures or warnings through `Msg::LaunchExited` in the footer.
