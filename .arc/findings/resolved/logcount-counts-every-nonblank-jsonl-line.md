---
id: logcount-counts-every-nonblank-jsonl-line
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/log.rs:255-277,289-295; src/commands/doctor.rs:274-317
- **reason:** `log::count` counts every nonblank JSONL line without parsing it, so corrupt records are included in Doctor's ordinary run count with no corruption disclosure.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: `log::count` now parses every record line separately and returns distinct parsed and unparseable counts, which Doctor displays.
