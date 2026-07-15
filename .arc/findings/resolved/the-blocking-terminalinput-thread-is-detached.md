---
id: the-blocking-terminalinput-thread-is-detached
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/commands/tui.rs:1262-1287,1305-1340
- **reason:** The blocking terminal-input thread is detached and neither cancelled nor joined before terminal restoration, so it can remain reading the shared terminal after the TUI returns.
- **rule:** restore-exclusive-resource-on-every-exit

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: The input thread now polls behind a shutdown flag and is explicitly joined before the caller restores the terminal.
