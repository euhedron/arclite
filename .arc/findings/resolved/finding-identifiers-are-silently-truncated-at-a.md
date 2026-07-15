---
id: finding-identifiers-are-silently-truncated-at-a
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/commands/promote.rs:180-215
- **reason:** Finding identifiers are silently truncated at a fixed 48-character cap that is neither configurable nor reported when applied.
- **rule:** no-silent-defaults

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

The silent half was the real defect: a curator saw a shortened id with no signal anything was cut. The cap itself exists for the platform path budget (documented on `SLUG_MAX_CHARS`).

## Next Action

The silence is fixed — truncation is now reported per promoted entry (human line and `truncated` in the JSON payload). The remaining ask, a *configurable* cap, is deliberately deferred: the constant is named, single-sourced, and documented with its rationale (the `no-hardcoded-magic-values` remediation bar), and no use has yet demanded tuning it. Revisit if description-based entry naming (the README's open item) or a real curation workflow needs longer ids.

## Resolution
Resolved per verify run `1784127749-81905-297967000`: Promotion now records truncation per entry through `Promoted.truncated` and marks it in human output; the remaining named 48-character limit is no longer silent.
