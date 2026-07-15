---
id: the-activerun-marker-stores-the-requested-model
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/runs.rs:110-123; src/commands/status.rs:18-24; src/commands/tui.rs:1759-1773
- **reason:** The active-run marker stores the requested model and both status views present it as the running model without marking that the backend has not confirmed its identity.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: Both status views now append the shared requested-model suffix to active-run model identities.
