---
id: stronger-run-ids
status: resolved
origin_kind: agent_session
system_run_id:
---

# Strengthen Run IDs

## Claim

Run ids based only on seconds plus process id are not strong enough for durable result lookup.

## Evidence

- `src/synth.rs` builds the run id from `now_secs()` and `std::process::id()`.
- `src/log.rs` stores the full run result by id, so an id collision can overwrite or confuse `arc log <id>` result lookup.
- Concurrent or quickly repeated processes are unlikely to collide in normal use, but pid reuse within the same second is a real OS-level possibility.

## Why It Matters

The log is the trace substrate for assessing spend, quality, and follow-up. Result identity should be boringly unique so future tooling can safely link findings, resolutions, and run artifacts.

## Next Action

Add subsecond time plus a local sequence or random component, then keep prefix lookup behavior intact. If changing historical ids is undesirable, only change newly created ids.

## Resolution
Resolved per verify run `1782660389-32544-91696900`: src/synth.rs now forms the id as `{ts}-{pid}-{now_subsec_nanos()}`, adding the subsecond entropy the finding asked for while resolve_id keeps prefix lookup intact.
