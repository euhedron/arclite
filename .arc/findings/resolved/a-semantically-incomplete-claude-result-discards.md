---
id: a-semantically-incomplete-claude-result-discards
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/ai.rs:244-251, 855-861
- **reason:** A semantically incomplete Claude result discards any usage, model, or cost fields that did parse and replaces them with zero tokens and unknown cost.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `parse_result` now preserves parsed model, token, and cost data when returning an error for an incomplete success payload.
