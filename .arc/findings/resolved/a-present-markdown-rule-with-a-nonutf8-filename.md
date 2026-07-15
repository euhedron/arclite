---
id: a-present-markdown-rule-with-a-nonutf8-filename
status: resolved
origin_kind: system_run
system_run_id: 1784088178-6661-783025000
commit: c7cb944
recorded: 2026-07-15 04:02 UTC
---

## Claim
- **location:** src/rules.rs:24-31
- **reason:** A present Markdown rule with a non-UTF-8 filename stem returns `Ok(None)` and disappears from the ruleset without an error or skipped-source disclosure.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784088178-6661-783025000` against commit `c7cb944` — see `arc log 1784088178-6661-783025000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784089595-30506-788658000`: `rule_from_file` now returns an error when a Markdown rule's filename stem is not valid UTF-8.
