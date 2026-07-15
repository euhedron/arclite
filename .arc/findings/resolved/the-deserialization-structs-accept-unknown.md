---
id: the-deserialization-structs-accept-unknown
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/settings.rs:40-75
- **reason:** The deserialization structs accept unknown fields, so misspelled settings such as a ruleset `source` key are silently ignored and can leave requested behavior inactive.
- **rule:** reject-unsupported-option-before-acting

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `Raw`, `RawDefaults`, `RawRuleset`, and `RawApiKeys` now use `serde(deny_unknown_fields)`.
