---
id: the-identical-fourfield-modelusage-summation
status: resolved
origin_kind: system_run
system_run_id: 1784199316-22990-696493000
commit: 04c4291
recorded: 2026-07-16 10:55 UTC
---

## Claim
- **location:** src/ai.rs — parse_result (the is_error Usage construction and the salvaged_tokens closure)
- **reason:** The identical four-field modelUsage summation (parsed.model_usage.values().map(|m| m.input_tokens/output_tokens/cache_creation_input_tokens/cache_read_input_tokens).sum()) is written out in full in two places, so a change to how modelUsage totals are computed must be made twice; a modelUsage-to-tuple helper would single-source it.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784199316-22990-696493000` against commit `04c4291` — see `arc log 1784199316-22990-696493000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784199527-31192-727813000`: src/ai.rs now has a single `model_usage_totals` helper computing all four sums, called by both the is_error path and the salvaged_tokens closure — the duplication is gone.
