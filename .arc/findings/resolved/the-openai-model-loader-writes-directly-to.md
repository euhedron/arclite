---
id: the-openai-model-loader-writes-directly-to
status: resolved
origin_kind: system_run
system_run_id: 1784128393-83999-871269000
commit: 50ae120
recorded: 2026-07-15 15:13 UTC
---

## Claim
- **location:** src/ai.rs:542-546; src/commands/tui.rs:523-539, 674-685
- **reason:** The OpenAI model loader writes directly to stderr while TUI worker threads call it, which can corrupt the terminal while the TUI owns it.
- **rule:** restore-exclusive-resource-on-every-exit

## Evidence
Promoted from `arc run audit` run `1784128393-83999-871269000` against commit `50ae120` — see `arc log 1784128393-83999-871269000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784129446-97257-143746000`: The model loader now returns caveats as `ModelListing` data without printing, and TUI message handlers render those caveats from state.
