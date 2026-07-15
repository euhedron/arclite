---
id: codex-run-records-set-usagemodel-from-the
status: resolved
origin_kind: system_run
system_run_id: 1784081993-17405-610330000
commit: cc6690c
recorded: 2026-07-15 02:19 UTC
---

## Claim
- **location:** src/ai.rs:CodexUsage::into_usage
- **reason:** Codex run records set `Usage.model` from the requested model because events do not echo a model id, but the report/log present it as the model that ran without disclosing that it is an unconfirmed fallback.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784081993-17405-610330000` against commit `cc6690c` — see `arc log 1784081993-17405-610330000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784082568-25953-890425000`: resolved: `Usage` now carries `model_source`, `CodexUsage::into_usage` marks Codex model identity as `Requested`, and the run report labels requested-only model ids as unconfirmed when the backend echoes no model id.
