---
id: both-newestrun-views-discard-the-returned
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/commands/log.rs:12-18; src/commands/tui.rs:1115-1117
- **reason:** Both newest-run views discard the returned unparseable-line count and can present an older parsed record as the newest result without disclosing log corruption.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: Both `arc log --last` and the TUI now retain and disclose the unparseable-line count.
