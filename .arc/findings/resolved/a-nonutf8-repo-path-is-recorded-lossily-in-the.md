---
id: a-nonutf8-repo-path-is-recorded-lossily-in-the
status: resolved
origin_kind: system_run
system_run_id: 1784081129-10121-597946000
commit: 414d9ba
recorded: 2026-07-15 02:05 UTC
---

## Claim
- **location:** src/log.rs:156-164, src/commands/promote.rs:47-54, src/commands/retire.rs:52-59
- **reason:** A non-UTF-8 repo path is recorded lossily in the run log and later reopened by `promote`/`retire`, so stored state can stop addressing the actual repository path.
- **rule:** confine-display-formatting-to-output

## Evidence
Promoted from `arc run audit` run `1784081129-10121-597946000` against commit `414d9ba` — see `arc log 1784081129-10121-597946000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784081733-16636-887670000`: Resolved: repo paths are now recorded through `log::repo_record_string`, which uses exact `to_str()` after `commands::resolve_root` rejects non-UTF-8 repository paths, rather than a lossy display conversion.
