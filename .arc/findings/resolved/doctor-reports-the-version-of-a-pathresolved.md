---
id: doctor-reports-the-version-of-a-pathresolved
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/commands/doctor.rs:94-114,283-287; src/http.rs:40,93-123
- **reason:** Doctor reports the version of a PATH-resolved `curl`, while outbound HTTP deliberately selects the canonical system curl first, so the report can identify a different executable from the one arclite uses.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: Doctor now uses `probe_curl`, which resolves the executable through the same `http::curl_program` function used for outbound HTTP.
