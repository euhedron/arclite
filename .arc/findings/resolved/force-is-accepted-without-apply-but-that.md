---
id: force-is-accepted-without-apply-but-that
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/cli.rs:156-162; src/commands/update.rs:40-45
- **reason:** `--force` is accepted without `--apply`, but that execution path never reads it and proceeds as an ordinary update check.
- **rule:** reject-unsupported-option-before-acting

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: Clap's `requires = "apply"` rejects `--force` without `--apply`.
