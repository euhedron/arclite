---
id: duplicate-rule-ids-across-sources-are
status: resolved
origin_kind: system_run
system_run_id: 1784081129-10121-597946000
commit: 414d9ba
recorded: 2026-07-15 02:05 UTC
---

## Claim
- **location:** src/rules.rs:62-78
- **reason:** Duplicate rule ids across sources are overwritten by `BTreeMap::insert` with later sources winning, but the override is not returned or surfaced to the caller, so an active ruleset can silently drop a rule body.
- **rule:** no-silent-defaults

## Evidence
Promoted from `arc run audit` run `1784081129-10121-597946000` against commit `414d9ba` — see `arc log 1784081129-10121-597946000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784081733-16636-887670000`: Resolved: `rules::load_sources` now records id collisions in an `overridden` list, and both rules reporting and synthesis context surface those overrides instead of silently dropping the replaced rule body.
