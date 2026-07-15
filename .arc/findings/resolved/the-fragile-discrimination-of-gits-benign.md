---
id: the-fragile-discrimination-of-gits-benign
status: resolved
origin_kind: system_run
system_run_id: 1784134448-45655-292448000
commit: 5a88654
recorded: 2026-07-15 16:54 UTC
---

## Claim
- **location:** src/commands/doctor.rs (git_repo_root) and src/synth.rs (repo_commit)
- **reason:** The fragile discrimination of git's benign not-in-a-repo verdict — forcing `LC_ALL=C` and matching `stderr.contains("not a git repository")` — is copy-implemented in two files instead of single-sourced, so if git's wording changes both must be found and fixed and can silently drift.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784134448-45655-292448000` against commit `5a88654` — see `arc log 1784134448-45655-292448000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784134736-50668-580271000`: doctor.rs::git_repo_root and synth.rs::repo_commit both delegate the not-a-repo stderr match to the single-sourced crate::git_stderr_says_not_a_repo helper in lib.rs, so the discrimination is no longer copy-implemented.
