---
id: srcairs-parseresult-the-let-model-models
status: resolved
origin_kind: system_run
system_run_id: 1784136986-90442-484292000
commit: 239aff6
recorded: 2026-07-15 17:36 UTC
---

## Claim
- **location:** src/ai.rs, parse_result — the `let (model, models, model_source) = match model { Some(reported) => (reported, models.clone(), ModelSource::Reported), None => (requested_model.to_owned(), vec![requested_model.to_owned()], ModelSource::Requested) }` block, appearing once in the `if parsed.is_error` error-payload path and again verbatim in the incomplete-success (`let (Some(text), ...) = complete else`) path
- **reason:** The identical fallback model-identity resolution match is copy-pasted in two branches of the same function, so a change to how an unconfirmed model resolves (e.g. the ModelSource semantics) must be made in both or they silently drift.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784136986-90442-484292000` against commit `239aff6` — see `arc log 1784136986-90442-484292000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784137275-94890-468643000`: The duplicated match is gone: both the error-payload and incomplete-success paths now call the shared `model_identity(&confirmed, requested_model)` helper (src/ai.rs), so the fallback resolution has a single home.
