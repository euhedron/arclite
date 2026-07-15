---
id: defaultisolated-claude-runs-disable-memory-and
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/ai.rs:739-773
- **reason:** Default-isolated Claude runs disable memory and inherited MCP servers but still inherit Claude user settings that can shape hooks and other behavior.
- **rule:** isolate-ambient-config-by-default

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: Default-isolated Claude invocations now pass `--setting-sources ""` in addition to disabling memory sources.
