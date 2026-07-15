---
id: the-agents-essential-prompt-write-discards-every
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/ai.rs:789-799,848-850
- **reason:** The agent's essential prompt write discards every `write_all` error as though it were best-effort bookkeeping, so completion does not confirm that the full prompt reached the child.
- **rule:** best-effort-side-effects

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: The prompt-writer thread now returns its write error through `Driven`, and a claimed backend success with a partial prompt is converted into an errored synthesis with usage preserved.
