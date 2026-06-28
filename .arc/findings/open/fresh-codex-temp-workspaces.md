---
id: fresh-codex-temp-workspaces
status: open
origin_kind: agent_session
system_run_id:
---

# Use Fresh Codex Temp Workspaces

## Claim

The Codex backend's temp directory naming can reuse an old directory if cleanup previously failed and a process id is reused, which risks trusting stale `out.txt` output.

## Evidence

- `src/ai.rs` `CodexTemp::new` uses a deterministic `arclite-codex-<pid>-<sequence>` directory under the system temp dir.
- It calls `create_dir_all`, which succeeds for an existing directory.
- `src/ai.rs` `Drop for CodexTemp` treats cleanup as best-effort and warns on failure, so leftovers are possible.
- `src/ai.rs` `synthesize_codex` later reads `out.txt` from that directory as the result artifact.

## Why It Matters

Generated result artifacts should be freshly owned by the current run. Best-effort cleanup is fine, but a later run should not be able to consume stale output left by a prior process.

## Next Action

Create an exclusive fresh temp directory or remove any existing candidate before use with collision handling. The important property is that a successful `CodexTemp::new` guarantees no stale `out.txt` can already exist.

## Resolution
