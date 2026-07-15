---
id: probe-and-probecurl-duplicate-the-same-spawn
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/commands/doctor.rs:101-143
- **reason:** `probe` and `probe_curl` duplicate the same spawn, exit-status, and version-output classification logic, differing only in command construction.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: `probe` and `probe_curl` now differ only in command resolution and share all spawn and version classification through `probe_version`.
