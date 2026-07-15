---
id: when-claude-reports-multiple-modelusage-entries
status: resolved
origin_kind: system_run
system_run_id: 1784113916-62001-655727000
commit: 6c86780
recorded: 2026-07-15 11:11 UTC
---

## Claim
- **location:** src/ai.rs::parse_result
- **reason:** When Claude reports multiple `modelUsage` entries, the code selects the largest-output entry and marks it `Reported` even though the payload confirms only which models ran, not which one produced the synthesis.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784113916-62001-655727000` against commit `6c86780` — see `arc log 1784113916-62001-655727000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784123334-47005-248915000`: `parse_result` now reports all confirmed model identities joined as a set instead of attributing the synthesis to one selected entry.
