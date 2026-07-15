---
id: an-eventread-error-silently-terminates-the-input
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/commands/tui.rs:1204-1212
- **reason:** An `event::read` error silently terminates the input thread while the tick sender keeps the TUI running as though input were merely idle.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: The input thread now sends `Msg::InputFailed`, causing the loop to exit and report the terminal-input error.
