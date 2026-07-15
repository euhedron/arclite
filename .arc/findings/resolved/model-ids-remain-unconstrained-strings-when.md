---
id: model-ids-remain-unconstrained-strings-when
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/commands/config.rs:39-52,103-110; src/ai.rs:863-871,1183-1184
- **reason:** Model ids remain unconstrained strings when emitted after the child CLI's `--model` option, allowing an option-shaped configured id to escape its value slot.
- **rule:** guard-values-interpolated-into-commands

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: Model ids are validated by `validate_model_id` both when set through config and again after final model resolution before constructing either backend command.
