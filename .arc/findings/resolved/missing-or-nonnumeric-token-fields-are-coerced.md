---
id: missing-or-nonnumeric-token-fields-are-coerced
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/log.rs:45-68
- **reason:** Missing or non-numeric token fields are coerced to zero, making malformed or partial usage records indistinguishable from genuine zero consumption.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: `usage_tokens` now counts malformed fields, and both rollup and stored-run views explicitly disclose them.
