---
id: both-ledger-commands-independently-reimplement
status: resolved
origin_kind: system_run
system_run_id: 1784134448-45655-292448000
commit: 5a88654
recorded: 2026-07-15 16:54 UTC
---

## Claim
- **location:** src/commands/promote.rs (run, ~L44-72) and src/commands/retire.rs (run, ~L36-60)
- **reason:** Both ledger commands independently re-implement the identical preamble — resolve the run-id prefix, load_stored, extract `command` and `repo` from the record, and `try_is_dir`-validate the repo still exists (with near-identical error wording) — rather than sharing one helper, so a change to how a stored run is located/validated must be edited in two places.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784134448-45655-292448000` against commit `5a88654` — see `arc log 1784134448-45655-292448000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784134736-50668-580271000`: Both promote.rs and retire.rs now call the shared crate::commands::log::stored_ledger_run, which single-sources the resolve-id/load_stored/extract-command-and-repo/try_is_dir preamble.
