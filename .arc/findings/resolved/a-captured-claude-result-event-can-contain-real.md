---
id: a-captured-claude-result-event-can-contain-real
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/ai.rs:967-985
- **reason:** A captured Claude `result` event can contain real usage, but any later stream/wait failure discards it and records unknown zero placeholders.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: The `DriveError::AfterSpawn` path now parses any captured `result_line` and preserves its usage before recording the stream failure.
