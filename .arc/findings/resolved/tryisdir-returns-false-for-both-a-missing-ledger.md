---
id: tryisdir-returns-false-for-both-a-missing-ledger
status: resolved
origin_kind: system_run
system_run_id: 1784113916-62001-655727000
commit: 6c86780
recorded: 2026-07-15 11:11 UTC
---

## Claim
- **location:** src/synth.rs::gather_findings
- **reason:** `try_is_dir` returns false for both a missing ledger and an existing non-directory ledger path, so a corrupt `.arc/findings/open` is silently treated as no findings.
- **rule:** distinguish-absent-from-unreadable

## Evidence
Promoted from `arc run audit` run `1784113916-62001-655727000` against commit `6c86780` — see `arc log 1784113916-62001-655727000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784123334-47005-248915000`: `gather_findings` now separately probes existence and errors when the ledger path exists but is not a directory.
