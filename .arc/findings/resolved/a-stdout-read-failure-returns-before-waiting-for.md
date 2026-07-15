---
id: a-stdout-read-failure-returns-before-waiting-for
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/ai.rs:660-667, 777-788
- **reason:** A stdout read failure returns before waiting for or terminating the spawned agent, allowing it to continue consuming tokens after unknown/zero usage is recorded.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: The stdout-read error path now kills and waits for the child before returning an accounted `AfterSpawn` error.
