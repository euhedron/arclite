---
id: claudes-ambientmemory-path-also-reenables
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/cli.rs:369-374; src/ai.rs:891-901
- **reason:** Claude's `--ambient-memory` path also re-enables user/project settings and hooks by omitting `--setting-sources ""`, although the option and run report disclose only ambient memory.
- **rule:** isolate-ambient-config-by-default

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: The option now explicitly discloses that Claude ambient mode restores CLAUDE.md, auto-memory, settings, and hooks, while the default path still suppresses all of them.
