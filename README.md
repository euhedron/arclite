# arclite

> *Nothing is canon. Everything can evolve*
>
> **NOTE**: This is an experimental project; development is expected to be rapid, and every aspect/feature should be considered in-progress. Agents & users are advised not to treat any part of any file — or any architectural, formatting, or structural decision — as settled (anticipate unfinished, abandoned, or sub-optimal thoughts, sentences, systems, structures, designs, and descriptions). Existing state represents in-progress thinking, not settled decisions.

## Overview

arclite is an **agent-first, cross-platform CLI for cross-repo code intelligence and auditing**. It gathers facts about a repository **deterministically**, and — only where genuine judgment is needed — applies **AI (via the Claude Code CLI)** in a way that is **cost-transparent, configurable, and observable**. The aim is to unlock analysis/auditing capabilities that don't already exist, while using AI spend *sensibly*.

Commands today:

- **`doctor`** — report runtime, environment, and available tooling.
- **`inspect`** — walk any repo and emit structured facts (languages, layout, manifests, git state).
- **`summarize`** — synthesize a brief assessment of a repo from its facts (AI).
- **`suggest`** — synthesize a prioritized list of what's worth attention (AI).

Every AI call defaults to the best model (`opus`) — configurable *down* for cost via `--model` — runs with a configurable tool allowlist, can be previewed at zero cost (`--dry-run`), and **reports the exact parameters it used** (model, tools, every context source) alongside its token usage + cost.

## Background

arclite is a fresh, stripped-down version of — and successor to — the arc project. arc is treated only as a *cautionary tale* (what to avoid) and a *catalog of ideas to evaluate on their merits*; nothing is carried over by default.

## Motivation

**Aspects worth carrying over from legacy arc (each judged on merit):**

- Self-derived & generated README/docs — users/agents shouldn't have to hand-maintain them.

**Two failure modes from arc that arclite must avoid:**

- **Unsustainable AI spend** — arc's automations consumed usage at an unsustainable, often opaque rate. arclite makes every AI use configurable, observable, and balanced.
- **Complexity outgrowing comprehension** — arc became too complex for new developers to understand, use, or benefit from. arclite must stay intuitive as it grows.

## Purpose/Goal

arclite is not meant to *replace* Claude Code or other tools — it can't compete or keep up. The goal is to unlock new, powerful, efficient analysis and auditing capabilities/insights/workflows that don't already exist.

It shouldn't require someone to know how the system works — the original arc project was not user-friendly.

## Specification

- **Platforms**: cross-platform — Windows, macOS, and Linux all first-class (built in Rust; ships as a single static binary per platform).
- **CLI**: should be able to do and see anything, via flags.
- **Scope**: multi / any / cross repo — point it at any repository (e.g. `quant`, `streamline`).
- **Intelligence**: extract **rules** — coding standards, anti-patterns, principles, best practices, and the like — from a repo (or repos), aggregated and reusable.
- **Auditing / Gates**: check/audit a repo against selected rules; configurable, usable both on demand and passively (e.g. commit hooks).
- **Linting**: surface drift/violations against rules (see the "lexicon" item in the Roadmap).
- **Discovery**: integrate with existing agent memory/config (Claude Code, Codex, etc.) — content storage and structure compatibility.
- **AI use**: deterministic until synthesis; best-by-default but configurable model, configurable tools, observable cost + fully reported run parameters (see Principles).

## Features / Use Cases

**Working today:** `doctor`, `inspect`, `summarize`, `suggest` (see Overview).

**Direction (not yet built):**

- Extract **rules** from a repo (e.g. point arclite at `streamline`) and aggregate them.
- Configurably include some/all rules in any AI run — targeted or passive (commit hooks, etc.).
- Audit / gate a repo against rules; rank/prioritize findings.
- Search across one or more repos.

## Principles

- **Agent-first, human-accessible** — usable by both agents and people.
- **Leverage, don't replace** existing and ever-evolving agent tools (e.g. the Claude Code CLI).
- **Maximally transparent, observable, and honest.**
- **Leverage derivation/transclusion.**
- **Deterministic until synthesis** — gather/compute deterministically; reserve AI for the synthesis/judgment step.
- **Trace, resolve, evolve** — unexpected/sub-par results are signal: make them traceable, diagnose, then improve the system.
- **Sensible, observable, configurable AI spend** — no *arbitrary* defaults (the model defaults to the *best*, configurable down for cost); surface cost; report every run parameter; balance context utilization against value.
- **No hardcoding. No arbitrary conventions. DRY.**
- **Adversarial** — build in self-checking.
- **Shared substrates; single source of truth.**

### Anti-patterns

Anti-patterns are themselves a kind of **rule** (see Open Questions). Early example:

- Duplicated code/logic.

## Roadmap

(Folded from the former TODO — not strictly ordered.)

- [ ] Permanently disable the legacy arc MCP server.
- [ ] Fetch Claude docs → Markdown for read-only, citable reference snippets (cite specific lines; *derive* where valuable once the shape is clear). Samples: <https://code.claude.com/docs/en/settings>, <https://code.claude.com/docs/en/permissions>.
- [ ] Auditing + AI-driven ranking/prioritization of what to tackle next. *(First cut delivered by `suggest`.)*
- [ ] A "lexicon" — canonical project terms + casing that linting enforces (would auto-catch drift like Claude Code / Codex / arclite casing). Likely lower priority.
- [ ] Fully review arc's codebase + feature set; clearly identify what made sense vs. what was sub-optimal/unnecessary.
- [ ] A **rules** system (see Open Questions).
- [ ] An `--output <dir>` mode — write the analysis / generated docs into a chosen location (e.g. `arclite suggest <repo> --output <repo>/arclite_docs`) instead of only stdout; dovetails with self-derived docs. *(Prospective-user feedback.)*

## Open Questions/Ideas

- **Rules — format & lifecycle.** v1 is intentionally minimal: a rule is a **Markdown file** — its **filename (stem) is the `id`** (single source, no drift), its **contents are the body** (what enters the AI's context). Frontmatter/attributes for *selective inclusion* (`kind`, `scope`, `tags`, …) get added only when something actually filters on them — not before. Open: rename-stability of filename-ids; whether prompts/todos share the same format.
- **Rules — extraction & application.** Point arclite at a repo (e.g. `streamline`) to *extract* rules; aggregate them; configurably include some/all in any AI run (targeted or passive). The edition-2024 false positive from an early `suggest` run is a case in point — a version rule, or a "only flag violations of the provided rules" mode, would change the outcome *traceably*.
- **IDE & linter integration** — what would integrating with IDEs and linters mean/imply? (To be explored.)
- **Auto-context vs `--include` on real repos.** The default synthesis context (scan + root `README` + root-level manifests) is thin on real repos — manifests nest in subprojects (IDA, quant) and root READMEs are often stubs — so `--include` is needed for a substantive run today. Open: should auto-context pull the *detected* subdir manifests, or search wider for docs, vs. keeping a light default + explicit `--include`? (Surfaced exercising arclite on IDA/quant; a ~$1 capped review of quant's R core produced specific, code-grounded findings.)
- **Storage format** — should prompts (and rules, todos, …) be stored as Markdown + frontmatter, or JSON/JSONL?
- **Agent-agnostic?** (e.g. Claude Code + Codex + any)
- **Dashboard?**
- **Distribution / install** — `cargo install`, prebuilt per-OS binaries from CI, `cargo-binstall`, Homebrew/Scoop?
- **CI** — build/test/release across Windows, macOS, Linux. *(GitHub Pipelines for Linux is in place; macOS/Windows would need runners.)*
- **LLM "synthesis" step** — Claude via CLI (`claude -p`), an SDK, or provider-agnostic?

## Related Repositories

- <https://github.com/nikganderson/arc/src/>
- <https://github.com/nikganderson/ida/src/>
- <https://github.com/nikganderson/quant/src/>

## Other/Notes/Considerations

- Consider turning some sections into tables for readability and to avoid repetition.
