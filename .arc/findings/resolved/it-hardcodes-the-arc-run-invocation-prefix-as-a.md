---
id: it-hardcodes-the-arc-run-invocation-prefix-as-a
status: resolved
origin_kind: system_run
system_run_id: 1784199836-33678-543377000
commit: 42d5e36
recorded: 2026-07-16 11:03 UTC
---

## Claim
- **location:** src/synth.rs — gather_runs, the `## run {id} — `arc run {command}`` context/header format string
- **reason:** It hardcodes the `arc run` invocation prefix as a literal instead of deriving it from the single-sourced `cli::binary_name()`/`cli::NAME_RUN` (the exact string promote.rs's `run_invocation` builds), so a binary or run-group rename silently drifts this text out of sync with the one authoritative home.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784199836-33678-543377000` against commit `42d5e36` — see `arc log 1784199836-33678-543377000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784200016-41698-196030000`: gather_runs in src/synth.rs now builds the header via crate::commands::promote::run_invocation(&command) (single-sourced from cli::binary_name()/cli::NAME_RUN), with a comment explicitly noting it is never a hardcoded `arc run` literal — the drift the finding described is gone.
