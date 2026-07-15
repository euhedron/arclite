---
id: schemamismatch-problems-are-capped-at-5-and
status: resolved
origin_kind: system_run
system_run_id: 1784139670-14713-147631000
commit: 3babefc
recorded: 2026-07-15 18:21 UTC
---

## Claim
- **location:** src/synth.rs — `run`, the local schema re-check block (`problems.truncate(5)`)
- **reason:** Schema-mismatch problems are capped at 5 and joined into the error with no "…and N more" disclosure, silently eliding further drift details — contrary to the codebase's own disclosed-elision standard used everywhere else (e.g. `resolve_id`'s "… and N more", `top_ranked`'s "+N more", walk-error counts).
- **rule:** no-silent-defaults

## Evidence
Promoted from `arc run audit` run `1784139670-14713-147631000` against commit `3babefc` — see `arc log 1784139670-14713-147631000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784139853-18823-759733000`: The re-check block in src/synth.rs now computes `elided = total - problems.len()` after `truncate(5)` and appends `"; … and {elided} more"` when elided > 0, so the elision is disclosed rather than silent.
