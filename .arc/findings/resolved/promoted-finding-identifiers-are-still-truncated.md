---
id: promoted-finding-identifiers-are-still-truncated
status: superseded
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/commands/promote.rs:199-209
- **reason:** Promoted finding identifiers are still truncated at a fixed 48-character limit with no flag or setting through which callers can choose the cap.
- **rule:** no-silent-defaults

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Superseded by the earlier open entry `finding-identifiers-are-silently-truncated-at-a`, which carries the standing judgment note — this re-promotion records the same issue from a later audit (the README's open "same-issue supersession on re-promote" edge, resolved by hand until promote learns to do it).
