---
id: a-missing-provider-created-field-silently
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/ai.rs:486-509
- **reason:** A missing provider `created` field silently becomes zero and changes model-list ordering without any disclosure that the ordering used a fabricated timestamp.
- **rule:** no-silent-defaults

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: `created` is now optional; undated models are counted, disclosed, and sorted after dated entries without fabricating a timestamp.
