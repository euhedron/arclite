---
id: errors-while-clearing-repositioning-showing-the
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/commands/tui.rs:1247-1251
- **reason:** Errors while clearing, repositioning, showing the cursor, and flushing during terminal cleanup are all discarded without the warning required for failed best-effort side effects.
- **rule:** best-effort-side-effects

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: Terminal cleanup failures are collected and reported after restoration in a single warning.
