---
id: runs-that-started-without-returning-usage-are
status: resolved
origin_kind: system_run
system_run_id: 1784090217-31798-632772000
commit: 0d430c6
recorded: 2026-07-15 04:36 UTC
---

## Claim
- **location:** src/ai.rs:340-360; src/log.rs:34-37; src/commands/usage.rs:105-127,189-192
- **reason:** Runs that started without returning usage are recorded as zero tokens and then classified as ordinary token-only Codex runs, causing aggregate totals to understate unknown consumed spend.
- **rule:** account-for-consumed-cost-on-failure

## Evidence
Promoted from `arc run audit` run `1784090217-31798-632772000` against commit `0d430c6` — see `arc log 1784090217-31798-632772000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784113494-61262-820192000`: No-usage runs now carry `spend_unknown`, are excluded from measured sums and token-only counts, and receive a separate disclosure.
