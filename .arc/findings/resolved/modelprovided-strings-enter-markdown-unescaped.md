---
id: modelprovided-strings-enter-markdown-unescaped
status: resolved
origin_kind: system_run
system_run_id: 1784113916-62001-655727000
commit: 6c86780
recorded: 2026-07-15 11:11 UTC
---

## Claim
- **location:** src/synth.rs::item_bullets, src/commands/promote.rs::entry_md, and src/commands/retire.rs::mark_resolved
- **reason:** Model-provided strings enter Markdown unescaped and are later parsed by the exact `## Resolution` heading, allowing an embedded newline and heading to redirect the structural update.
- **rule:** guard-values-interpolated-into-commands

## Evidence
Promoted from `arc run audit` run `1784113916-62001-655727000` against commit `6c86780` — see `arc log 1784113916-62001-655727000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784123334-47005-248915000`: Promotion and retirement now pass model-provided text through `escape_ledger_text`, neutralizing line-leading Markdown headings.
