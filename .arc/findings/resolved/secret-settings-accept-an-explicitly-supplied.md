---
id: secret-settings-accept-an-explicitly-supplied
status: resolved
origin_kind: system_run
system_run_id: 1784085907-61326-336795000
commit: 708ab00
recorded: 2026-07-15 03:25 UTC
---

## Claim
- **location:** src/commands/config.rs:203-210
- **reason:** Secret settings accept an explicitly supplied value from CLI argv even though only the omitted-value path uses stdin.
- **rule:** keep-secrets-out-of-process-arguments

## Evidence
Promoted from `arc run audit` run `1784085907-61326-336795000` against commit `708ab00` — see `arc log 1784085907-61326-336795000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784086760-77273-560828000`: `config set` now rejects inline values for `api_keys.*` and accepts secret values only through stdin.
