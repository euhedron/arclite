---
id: the-docblocks-and-inline-comments-repeatedly
status: open
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/commands/tui.rs:1684-1714,1729-1746,1779-1781
- **reason:** The docblocks and inline comments repeatedly narrate column identities, layout arithmetic, and the immediately following branches already evident from the code.
- **rule:** prefer-concision

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

Narration that restates visible code is drift surface (prefer-concision); the status-table region had accumulated it.

## Next Action

Two trim passes landed (column-width tables, render_status header/tail comments, the requested-suffix notes). The verifier still judges the region narration-heavy; the residue is a style-judgment gap between arclite's deliberately rationale-dense comment convention and the rule's strictest reading. Take a dedicated comment-style pass over `tui.rs` with fresh eyes rather than salami-slicing further tonight — and if that pass concludes the remaining comments are load-bearing rationale, retire this with that note.

## Resolution
