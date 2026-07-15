---
id: the-identical-if-modelrequestedr-format
status: resolved
origin_kind: system_run
system_run_id: 1784132575-25930-180862000
commit: 96c5e64
recorded: 2026-07-15 16:22 UTC
---

## Claim
- **location:** src/commands/log.rs — the requested-model display block, duplicated in `row()` and in `stored_human()`
- **reason:** The identical `if model_requested(r) { format!("{}{}", field(r,"model"), MODEL_REQUESTED_SUFFIX) } else { field(r,"model") }` formatting logic is copy-pasted in both functions instead of a single shared helper (e.g. `log::model_display(&Value)`), so the two can drift even though the suffix constant itself is single-sourced.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784132575-25930-180862000` against commit `96c5e64` — see `arc log 1784132575-25930-180862000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784134110-43911-80458000`: log.rs defines the shared model_display(record) helper and both row() and stored_human() call it instead of inlining the requested-suffix formatting, removing the duplication.
