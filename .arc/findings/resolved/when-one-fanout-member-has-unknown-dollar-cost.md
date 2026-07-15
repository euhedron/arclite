---
id: when-one-fanout-member-has-unknown-dollar-cost
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/synth.rs:1533-1554
- **reason:** When one fan-out member has unknown dollar cost and another reports cost, the unknown member is added as zero and the resulting lower bound is presented as an exact total.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: `sum_usage` now marks mixed known and unknown costs as partial for rendering with a lower-bound indicator.
