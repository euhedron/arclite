---
id: the-documented-displayform-derivation-of
status: resolved
origin_kind: system_run
system_run_id: 1784136986-90442-484292000
commit: 239aff6
recorded: 2026-07-15 17:36 UTC
---

## Claim
- **location:** src/ai.rs `parse_result` (`models.join(" + ")`) and src/synth.rs `sum_usage` (`total.model = total.models.join(" + ")`)
- **reason:** The documented display-form derivation of `Usage::model` (members joined with ' + ') is open-coded in two files with no shared helper, so the separator/format has two homes despite `Usage::model`'s doc stating it as one fact.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784136986-90442-484292000` against commit `239aff6` — see `arc log 1784136986-90442-484292000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784137275-94890-468643000`: The `" + "` join now lives once in `ai::join_models`, and both call sites use it — parse_result via `model_identity` (`join_models(confirmed)`) and synth.rs sum_usage (`total.model = crate::ai::join_models(&total.models)`) — so the separator no longer has two homes.
