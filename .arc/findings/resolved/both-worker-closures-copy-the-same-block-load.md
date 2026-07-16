---
id: both-worker-closures-copy-the-same-block-load
status: resolved
origin_kind: system_run
system_run_id: 1784198798-10489-850735000
commit: ffd0f33
recorded: 2026-07-16 10:46 UTC
---

## Claim
- **location:** src/commands/tui.rs — App::spawn_models_fetch and the fetch worker closure inside App::cycle_launch_model
- **reason:** Both worker closures copy the same block — load Settings for cwd, call backend.list_models, and map the result to (Vec<String> ids, bool truncated, usize undated) with identical error-mapping — rather than sharing one helper, so the two provider-listing fetch paths can drift.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784198798-10489-850735000` against commit `ffd0f33` — see `arc log 1784198798-10489-850735000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784199120-21126-493940000`: Both `spawn_models_fetch` and `cycle_launch_model`'s Fetch worker now call the single shared helper `fetch_model_ids(cwd, backend)`, which owns the load-Settings/list_models/map-to-(ids,truncated,undated) block, so the duplication no longer exists.
