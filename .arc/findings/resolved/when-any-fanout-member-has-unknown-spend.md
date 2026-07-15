---
id: when-any-fanout-member-has-unknown-spend
status: resolved
origin_kind: system_run
system_run_id: 1784113916-62001-655727000
commit: 6c86780
recorded: 2026-07-15 11:11 UTC
---

## Claim
- **location:** src/commands/usage.rs::rollup and src/synth.rs::sum_usage
- **reason:** When any fan-out member has unknown spend, `sum_usage` marks the aggregate `spend_unknown` and `rollup` skips the entire aggregate, discarding known tokens and cost from successful members.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784113916-62001-655727000` against commit `6c86780` — see `arc log 1784113916-62001-655727000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784123334-47005-248915000`: The rollup now sums known token and cost fields even when `spend_unknown` is set, while separately disclosing the aggregate as a lower bound.
