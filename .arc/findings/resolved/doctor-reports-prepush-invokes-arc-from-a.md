---
id: doctor-reports-prepush-invokes-arc-from-a
status: resolved
origin_kind: system_run
system_run_id: 1784081129-10121-597946000
commit: 414d9ba
recorded: 2026-07-15 02:05 UTC
---

## Claim
- **location:** src/commands/doctor.rs:218-220 and src/commands/doctor.rs:236-244
- **reason:** `doctor` reports `pre-push invokes arc` from a substring heuristic that also matches comments or echoed text, so an inconclusive hook inspection can produce an all-clear gate verdict.
- **rule:** inconclusive-check-must-not-report-all-clear

## Evidence
Promoted from `arc run audit` run `1784081129-10121-597946000` against commit `414d9ba` — see `arc log 1784081129-10121-597946000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784081733-16636-887670000`: Resolved: `hook_invokes` now drops comment lines and requires the binary name at a word boundary followed by whitespace, so the cited substring/comment all-clear mechanism is no longer present.
