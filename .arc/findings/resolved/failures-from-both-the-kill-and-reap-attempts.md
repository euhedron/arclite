---
id: failures-from-both-the-kill-and-reap-attempts
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/ai.rs:773-781
- **reason:** Failures from both the kill and reap attempts are discarded, so a metered child may remain active after its run records failure without that continuing consumption being reported.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: Kill and reap failures are now appended to the stream error, which flows into the recorded errored synthesis.
