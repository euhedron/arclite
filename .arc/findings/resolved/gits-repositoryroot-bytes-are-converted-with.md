---
id: gits-repositoryroot-bytes-are-converted-with
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/commands/doctor.rs:121-130,180-204
- **reason:** Git's repository-root bytes are converted with `from_utf8_lossy` and the display-mangled string is then reused as the filesystem path for config and hook lookups.
- **rule:** confine-display-formatting-to-output

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: `git_repo_root` now uses strict `String::from_utf8` and errors when the repository path cannot round-trip losslessly.
