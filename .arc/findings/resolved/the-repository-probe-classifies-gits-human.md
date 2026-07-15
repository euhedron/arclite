---
id: the-repository-probe-classifies-gits-human
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/synth.rs:619-646
- **reason:** The repository probe classifies Git's human stderr text without pinning the locale, so its `not a git repository` match is locale-dependent.
- **rule:** prefer-machine-readable-tool-output

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `repo_commit` now sets `LC_ALL=C` before matching Git's not-a-repository diagnostic.
