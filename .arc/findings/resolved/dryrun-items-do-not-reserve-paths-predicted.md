---
id: dryrun-items-do-not-reserve-paths-predicted
status: resolved
origin_kind: system_run
system_run_id: 1784113916-62001-655727000
commit: 6c86780
recorded: 2026-07-15 11:11 UTC
---

## Claim
- **location:** src/commands/promote.rs::run — dry-run branch
- **reason:** Dry-run items do not reserve paths predicted earlier in the batch, so colliding slugs preview the same destination while real atomic claims suffix later entries.
- **rule:** preview-must-share-execution-path

## Evidence
Promoted from `arc run audit` run `1784113916-62001-655727000` against commit `6c86780` — see `arc log 1784113916-62001-655727000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784123334-47005-248915000`: The dry-run branch now shares a reservation set and `preview_findings_entry` reserves each predicted path before processing the next item.
