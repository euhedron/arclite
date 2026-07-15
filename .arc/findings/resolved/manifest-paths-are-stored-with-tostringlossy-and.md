---
id: manifest-paths-are-stored-with-tostringlossy-and
status: resolved
origin_kind: system_run
system_run_id: 1784081129-10121-597946000
commit: 414d9ba
recorded: 2026-07-15 02:05 UTC
---

## Claim
- **location:** src/commands/inspect.rs:127 and src/synth.rs:730-733
- **reason:** Manifest paths are stored with `to_string_lossy()` and later rejoined to the repo root for file reads, so a non-UTF-8 manifest path can be corrupted before lookup.
- **rule:** confine-display-formatting-to-output

## Evidence
Promoted from `arc run audit` run `1784081129-10121-597946000` against commit `414d9ba` — see `arc log 1784081129-10121-597946000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784081733-16636-887670000`: Resolved: `InspectReport::manifest_paths` now stores only exact UTF-8 relative paths via `rel.to_str()`, counting non-UTF-8 manifest paths as walk errors, so lossy manifest paths are not later rejoined for reads.
