---
id: the-fanout-ceiling-is-fixed-at-eight-offers-no
status: open
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** src/synth.rs:14-18; src/commands/mod.rs:94-99; src/cli.rs:391-396
- **reason:** The fan-out ceiling is fixed at eight, offers no configuration lever, and is described in help only as a `small maximum` rather than exposing the applied value.
- **rule:** no-silent-defaults

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

An invisible bound would be a silent default; a wrong bound would cap consensus sampling someone needs.

## Next Action

The visibility half is fixed: `--runs`' help now derives the applied ceiling from `synth::MAX_RUNS` (the one definition), and an over-limit value is rejected naming it. The configurability half is deliberately deferred — the ceiling is concurrency/spend hygiene (eight concurrent agent processes at premium-model prices), documented at its constant, and no use has demanded more. Revisit if a real consensus workflow needs a higher fan-out; until then this stays open as a judgment, not a defect queue item.

## Resolution
