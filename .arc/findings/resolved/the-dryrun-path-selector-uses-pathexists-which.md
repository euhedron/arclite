---
id: the-dryrun-path-selector-uses-pathexists-which
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/lib.rs:65-71
- **reason:** The dry-run path selector uses `Path::exists`, which maps metadata errors to false and can report an unreadable occupied candidate as free.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `preview_findings_entry` now uses fallible `try_exists`, propagating metadata errors instead of treating them as a free candidate.
