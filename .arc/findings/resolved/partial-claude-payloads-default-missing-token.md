---
id: partial-claude-payloads-default-missing-token
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/ai.rs:155-175,207-236,305-333; src/commands/usage.rs:141-148,211-214
- **reason:** Partial Claude payloads default missing token counters to zero and leave missing dollar cost marked as known, after which the rollup misclassifies the run as Codex-style token-only spend.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: Incomplete Claude payloads now salvage available counters, mark absent authoritative usage as unknown, and the rollup distinguishes missing Claude cost from Codex's by-design token-only records.
