---
id: changed-parses-gits-nuldelimited-byte-output
status: resolved
origin_kind: system_run
system_run_id: 1784081129-10121-597946000
commit: 414d9ba
recorded: 2026-07-15 02:05 UTC
---

## Claim
- **location:** src/synth.rs:560-578
- **reason:** `--changed` parses git's NUL-delimited byte output through `String::from_utf8_lossy()` and then uses the resulting string in `root.join(path)`, so non-UTF-8 changed paths are silently mangled.
- **rule:** confine-display-formatting-to-output

## Evidence
Promoted from `arc run audit` run `1784081129-10121-597946000` against commit `414d9ba` — see `arc log 1784081129-10121-597946000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784081733-16636-887670000`: Resolved: `changed_files` now parses `git status --porcelain -z` as raw bytes, decodes each path with `std::str::from_utf8`, and counts undecodable paths as skipped instead of using `String::from_utf8_lossy()`.
