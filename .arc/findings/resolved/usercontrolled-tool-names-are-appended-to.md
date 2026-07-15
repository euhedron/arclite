---
id: usercontrolled-tool-names-are-appended-to
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/ai.rs:824-830
- **reason:** User-controlled tool names are appended to Claude's variadic option without rejecting option-like values, allowing a value beginning with `--` to escape its slot into Claude's argument grammar.
- **rule:** guard-values-interpolated-into-commands

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: `Backend::reject_unsupported` now rejects empty or leading-dash tool names before synthesis begins.
