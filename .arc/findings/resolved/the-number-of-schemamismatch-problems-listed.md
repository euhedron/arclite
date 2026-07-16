---
id: the-number-of-schemamismatch-problems-listed
status: resolved
origin_kind: system_run
system_run_id: 1784199316-22990-696493000
commit: 04c4291
recorded: 2026-07-16 10:55 UTC
---

## Claim
- **location:** src/synth.rs — run(), problems.truncate(5)
- **reason:** The number of schema-mismatch problems listed before elision is a bare, unexplained literal, whereas the codebase's own standard for such caps is a named+documented constant (e.g. AMBIGUOUS_LISTED = 8 in log.rs).
- **rule:** no-hardcoded-magic-values

## Evidence
Promoted from `arc run audit` run `1784199316-22990-696493000` against commit `04c4291` — see `arc log 1784199316-22990-696493000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784199527-31192-727813000`: src/synth.rs now defines the documented constant `SCHEMA_PROBLEMS_LISTED = 5` and uses it in `problems.truncate(SCHEMA_PROBLEMS_LISTED)` instead of a bare literal.
