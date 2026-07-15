---
id: fanout-accounting-copies-the-first-responses
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/synth.rs:1568-1594
- **reason:** Fan-out accounting copies the first response's model identity without checking later response-derived identities, so a mixed-model fan-out is reported as though one model handled every run.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: `sum_usage` now joins distinct model identities and lowers confidence to requested if any member is unconfirmed.
