---
id: logged-human-views-and-generateddocument
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/commands/log.rs:102-123, 292-311; src/synth.rs:1197-1210, 1567-1581
- **reason:** Logged human views and generated-document provenance print the model id without carrying `model_source`, so Codex's requested-but-unconfirmed id is presented as the model that ran.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: Logged rows, details, and generated-document provenance now mark `ModelSource::Requested` as requested.
