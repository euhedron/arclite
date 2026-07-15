---
id: the-acknowledged-substring-heuristic-can
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/commands/doctor.rs:218-250
- **reason:** The acknowledged substring heuristic can classify a quoted `"arc run"` string as an invocation, yet doctor reports the definitive all-clear state `InvokesArc`.
- **rule:** inconclusive-check-must-not-report-all-clear

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: Doctor's serialized and human verdicts now explicitly qualify invocation detection as a text scan.
