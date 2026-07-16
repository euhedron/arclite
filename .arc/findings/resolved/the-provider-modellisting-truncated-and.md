---
id: the-provider-modellisting-truncated-and
status: resolved
origin_kind: system_run
system_run_id: 1784227347-76672-814181000
commit: 95edf09
recorded: 2026-07-16 18:42 UTC
---

## Claim
- **location:** src/commands/tui.rs (Msg::ModelsFetched caveat build, and render_launch ModelsState::Fetched) and src/commands/models.rs (truncated/undated push)
- **reason:** The provider model-listing 'truncated' and 'undated-entries' disclosure branch is reimplemented in three places with already-divergent wording ('carry no `created` timestamp — sorted last, not as oldest' vs 'undated model(s) sorted last' vs 'undated entr(y/ies) sorted last'), instead of one shared formatter like the codebase's own unreadable_entries/pruned_entries/prune_failed_entries helpers — the exact drift the rule warns of, and already drifted.
- **rule:** no-duplicated-logic

## Evidence
Promoted from `arc run audit` run `1784227347-76672-814181000` against commit `95edf09` — see `arc log 1784227347-76672-814181000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784227499-85989-407505000`: ai::listing_caveats(truncated, undated) is now the single shared formatter, called by models.rs and both tui.rs sites (Msg::ModelsFetched and render_launch's ModelsState::Fetched), eliminating the three divergent reimplementations.
