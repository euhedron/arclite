---
id: a-config-model-fetch-is-identified-only-by
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/commands/tui.rs:597-613, 1251-1284
- **reason:** A config model fetch is identified only by setting name, so a canceled fetch can be applied to a newer edit of the same setting.
- **rule:** tolerate-stale-async-results

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: Config model fetches now carry a generation and are applied only when it matches the currently open `ConfigEdit::Fetching`.
