---
id: dry-runs-always-preview-the-unsuffixed
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/commands/promote.rs:82-97; src/commands/retire.rs:110-117
- **reason:** Dry runs always preview the unsuffixed destination, while execution uses collision-aware claiming and may write a different suffixed path.
- **rule:** preview-must-share-execution-path

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: Promote and retire dry runs now use the same `findings_entry_candidates` sequence as execution claims.
