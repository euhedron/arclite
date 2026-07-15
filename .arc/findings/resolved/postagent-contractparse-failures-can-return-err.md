---
id: postagent-contractparse-failures-can-return-err
status: resolved
origin_kind: system_run
system_run_id: 1784081993-17405-610330000
commit: cc6690c
recorded: 2026-07-15 02:19 UTC
---

## Claim
- **location:** src/synth.rs:run / src/ai.rs:parse_result and synthesize_codex
- **reason:** Post-agent contract/parse failures can return `Err` after the child process ran, and `synth::run` propagates that before the logging path, so consumed tokens can be dropped unless the backend packaged the failure as `Synthesis.error`.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784081993-17405-610330000` against commit `cc6690c` — see `arc log 1784081993-17405-610330000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784082568-25953-890425000`: resolved: backend result/parse/artifact failures after the child runs are now converted into `Synthesis.error` with captured or explicitly unknown usage, and `synth::run` logs errored syntheses through the normal record path.
