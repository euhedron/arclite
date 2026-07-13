# Findings Ledger

This directory is a curated signal ledger for future human and agent sessions. It is not a transcript store and not a raw model-output dump. Promote only observations that have been judged useful enough to carry across contexts.

The format is intentionally provisional. Change the structure when experience shows a better one; keep the distinction between source kinds sharper than the exact file shape.

## Provenance Boundaries

Do not ask agents to authoritatively record who they are, which model they are, or what time it is. Agent-written provenance is only a hint and should never be treated as an audit trail.

Prefer durable provenance that the system records or derives: git history, `arc` run ids, future `arc` promotion metadata, issue ids, or explicit human review. In hand-authored finding files, keep provenance minimal: classify the source kind, and link to durable records when they exist.

## Source Kinds

- `agent_session`: live/dynamic work from a chat-style agent session, such as Claude Code, Codex, or a human-agent pairing. These are observations to verify, not system verdicts.
- `system_run`: a finding promoted from arclite itself, such as an `arc run audit` or `arc run critique` result. Include the run id when available.
- `human`: a human-authored observation, decision, or prioritization.
- `mixed`: a finding whose claim depends on more than one source kind.

Do not collapse these. A system-run finding has different provenance from an agent-session hypothesis, even when both point at the same code.

## Statuses

- `open`: worth carrying forward; not yet resolved.
- `accepted`: agreed as real, but not yet fixed.
- `resolved`: fixed or intentionally retired; include the commit/run/context that closed it.
- `rejected`: investigated and found not useful or not true.
- `superseded`: replaced by another finding, design, or rule.

## File Shape

Use one Markdown file per curated finding, usually under `open/` while it is active. Start with simple frontmatter so later tooling can parse it without guessing:

```yaml
---
id: short-kebab-case-id
status: open
origin_kind: agent_session
system_run_id:
---
```

Then use short sections:

- `Claim`: the finding in one paragraph.
- `Evidence`: concrete file/line references, commands, or run ids.
- `Why It Matters`: the risk or leverage.
- `Next Action`: what a future agent should verify or do.
- `Resolution`: leave blank until closed.

Keep raw transcripts, full model outputs, and speculative brainstorming out of this directory. Link to run ids or summarize the relevant evidence instead.

Ground the Claim/Evidence in the concrete mechanism — the file, line, and construct — wherever one exists: `arc run verify` judges an entry by whether the cited mechanism still exists in the current code, so a mechanism-grounded entry stays in the automated verify/retire lifecycle regardless of its `origin_kind`, while a behavior-only claim ("the site does X when…") can only come back `indeterminate`. (Proven 2026-07-12: four hand-authored `agent_session` entries about a deployed site verified `reproduces` because each cited the code behind the behavior.)

## Open Edges

- The directory name, frontmatter keys, and lifecycle are not canon.
- Future `arc` commands or TUI flows may promote, verify, deduplicate, and retire findings directly.
- If findings become numerous, split by status and scope before adding heavier machinery.
