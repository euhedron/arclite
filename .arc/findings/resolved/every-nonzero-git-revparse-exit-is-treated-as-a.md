---
id: every-nonzero-git-revparse-exit-is-treated-as-a
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/synth.rs:615-634
- **reason:** Every nonzero `git rev-parse` exit is treated as a benign absent commit, collapsing repository corruption or other Git failures into the not-a-repository/unborn-HEAD case.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: `repo_commit` now treats only unborn HEAD or not-a-repository as benign and warns on other failures.
