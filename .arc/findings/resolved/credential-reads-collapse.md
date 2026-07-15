---
id: credential-reads-collapse
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/ai.rs:699-708; src/commands/update.rs:92-94
- **reason:** Credential reads collapse `std::env::VarError::NotUnicode` into the absent case, silently falling back to saved credentials or unauthenticated access.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: Both `provider_key` and the updater’s token loader now handle `VarError::NotUnicode` as an explicit error instead of absence.
