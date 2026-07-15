---
id: the-windows-liveness-probe-requests-csv-but
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/runs.rs:179-186
- **reason:** The Windows liveness probe requests CSV but parses it with raw comma splitting, so a quoted comma in the first field shifts the PID column and can falsely prune a live marker.
- **rule:** prefer-machine-readable-tool-output

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: The Windows probe now splits on the quoted CSV field boundary `","`, so commas inside the quoted image-name field do not shift the PID field.
