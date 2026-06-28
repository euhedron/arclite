---
id: fail-closed-multi-run-gates
status: resolved
origin_kind: agent_session
system_run_id:
---

# Fail Closed For Partial Multi-Run Gates

## Claim

`arc run ... --fail-on-findings --runs N` can pass when one or more child runs fail, as long as at least one surviving run succeeds and reports no findings.

## Evidence

- `src/synth.rs` `multi_synthesize` skips failed child runs and only errors when all runs fail.
- `src/synth.rs` `run` computes the gate outcome from the combined surviving result.
- The latest Codex critique in local run `1782170917-42028` independently flagged this as a gate-risk pattern.

## Why It Matters

A gate is an enforcement surface. Partial execution means incomplete enforcement, so a clean surviving result should not be treated the same as a fully successful clean run.

## Next Action

Decide whether gated multi-run failures should exit as an arclite error or as a blocked gate. Then change `multi_synthesize` or the gate path so partial failure cannot silently pass, and add a deterministic test around the policy if the code shape allows it.

## Resolution
Resolved per verify run `1782660389-32544-91696900`: src/synth.rs run() now sets `incomplete_gate = opts.gate.is_some() && runs < opts.runs` and ORs it into `gate_blocked`, so a gated fan-out that lost any run blocks even when survivors are clean.
