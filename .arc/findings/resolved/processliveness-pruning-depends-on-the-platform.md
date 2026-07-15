---
id: processliveness-pruning-depends-on-the-platform
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/runs.rs:157-175
- **reason:** Process-liveness pruning depends on the platform `kill` or `tasklist` implementation but resolves that executable through PATH rather than a trusted system path.
- **rule:** resolve-trusted-tool-by-explicit-path

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: The Unix probe resolves canonical `/bin/kill` or `/usr/bin/kill`, while Windows constructs `System32/tasklist.exe` from `SystemRoot`.
