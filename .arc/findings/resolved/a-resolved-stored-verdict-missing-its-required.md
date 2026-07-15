---
id: a-resolved-stored-verdict-missing-its-required
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/commands/retire.rs:84-110
- **reason:** A resolved stored verdict missing its required `id` is silently skipped and a missing `reason` becomes empty, so malformed data masquerades as no actionable verdict.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `retire` now rejects resolved verdicts lacking a nonempty `id` or a string `reason`.
