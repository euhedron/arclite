---
id: the-runnable-arc-run-command-invocation-string
status: resolved
origin_kind: system_run
system_run_id: 1784139670-14713-147631000
commit: 3babefc
recorded: 2026-07-15 18:21 UTC
---

## Claim
- **location:** src/commands/promote.rs — `run` (the head-summary `invocation`) and `entry_md` (the evidence-line `invocation`)
- **reason:** The runnable `arc run <command>` invocation string is built by a byte-identical `format!("{} {} {command}", crate::cli::binary_name(), crate::cli::NAME_RUN)` in two separate functions in the same module, so one can silently drift from the other despite a single natural home.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784139670-14713-147631000` against commit `3babefc` — see `arc log 1784139670-14713-147631000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784139853-18823-759733000`: promote.rs now defines a single `run_invocation(command)` helper building `format!("{} {} {command}", binary_name(), NAME_RUN)`, and both the head-summary `run` and `entry_md` call it, so the byte-identical duplication is gone.
