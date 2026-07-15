---
id: backgroundrun-warnings-exist-only-in-captured
status: resolved
origin_kind: system_run
system_run_id: 1784113916-62001-655727000
commit: 6c86780
recorded: 2026-07-15 11:11 UTC
---

## Claim
- **location:** src/commands/tui.rs::App::confirm_launch
- **reason:** Background-run warnings exist only in captured stderr forwarded through a channel whose send failure is discarded, so quitting the TUI before completion silently loses auxiliary-write failures.
- **rule:** best-effort-side-effects

## Evidence
Promoted from `arc run audit` run `1784113916-62001-655727000` against commit `6c86780` — see `arc log 1784113916-62001-655727000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784123334-47005-248915000`: `confirm_launch` now falls back to `eprintln!` when the TUI receiver is gone, preserving background-run warnings after exit.
