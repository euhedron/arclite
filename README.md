# arclite

> *Nothing is canon. Everything can evolve.*
>
> **NOTE**: This is an experimental project; development is expected to be rapid, and every aspect/feature should be considered in-progress. Agents & users are advised not to treat any part of any file — or any architectural, formatting, or structural decision — as settled (anticipate unfinished, abandoned, or sub-optimal thoughts, sentences, systems, structures, designs, and descriptions). Existing state represents in-progress thinking, not settled decisions.

## Overview

arclite is an **agent-first, cross-platform CLI for cross-repo code intelligence and auditing**. It gathers facts about a repository **deterministically**, and — only where genuine judgment is needed — applies **AI (via the Claude Code CLI)** in a way that is **cost-transparent, configurable, and observable**. The aim is to unlock analysis/auditing capabilities that don't already exist, while using AI spend *sensibly*.

Commands today:

- **`doctor`** — report runtime, environment, and available tooling.
- **`inspect`** — walk any repo and emit structured facts (languages, layout, manifests, git state).
- **`summarize`** — synthesize a brief assessment of a repo from its facts (AI).
- **`suggest`** — synthesize a prioritized list of what's worth attention (AI).
- **`extract`** — synthesize reusable rules (standards, anti-patterns, principles) from a repo, as candidates to curate (AI).
- **`audit`** — check a repo against selected rules, reporting only concrete violations (AI).

Every AI call defaults to the best model (`opus`) — configurable *down* for cost via `--model` — runs with a configurable tool allowlist, can be previewed at zero cost (`--dry-run`), and **reports the exact parameters it used** (model, tools, every context source) alongside its token usage + cost.

## Getting started

**Prerequisites:** a Rust toolchain (`cargo`); and, for the AI commands (`summarize`/`suggest`/`extract`/`audit`), the Claude Code CLI installed and authenticated (`claude` on `PATH`). `arc doctor` verifies both.

**Build / install:**

```sh
git clone https://github.com/nikganderson/arclite.git && cd arclite
cargo install --path .        # installs the `arc` command; or `cargo build --release` → target/release/arc
```

**Use** (each command takes a repo path, default `.`):

```sh
arc                                # self-documenting help (the binary is `arc`; the project is arclite)
arc doctor                         # runtime + tooling check (free)
arc inspect <repo>                 # deterministic facts, no AI (free)
arc suggest <repo> --include src   # AI review — preview with --dry-run ($0) first
arc audit   <repo> --rules rules   # flag only violations of the given rules
arc extract <repo> --include src   # propose reusable rules to curate into rules/
```

Deterministic commands cost nothing; the AI commands print the model, tools, every context source, and exact token usage + cost, and `--dry-run` previews the prompt at zero spend. Any synthesis command can scope its context to just your git-changed files with `--changed` — a cheap, focused run (e.g. `arc audit --rules rules --changed`) — and `--output <dir>` also saves the result as a self-describing Markdown doc.

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
- **AI use**: deterministic until synthesis; best-by-default but configurable model, configurable tools, observable cost + fully reported run parameters (see Principles). The synthesis subprocess runs with ambient memory disabled (no user/project `CLAUDE.md` or auto-memory auto-loaded), so the reported context is authoritative and runs reproduce across machines.

## Features / Use Cases

**Working today:** `doctor`, `inspect`, `summarize`, `suggest`, `extract`, `audit` (see Overview).

**Direction (not yet built):**

- Aggregate extracted **rules** across repos and dedup them (`extract` produces per-repo candidates today; cross-repo merge is the open part).
- Configurably include some/all rules in any AI run — targeted or passive (commit hooks, etc.).
- Gate a repo against rules *passively* (e.g. commit hooks) and rank/prioritize findings — `audit` flags violations on demand today; the passive + ranking parts are open.
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

## Roadmap

Open and unsettled — not a plan, an ordering, or a commitment; like everything here it evolves (items get added, dropped, or reshaped as signals warrant).

- [ ] Permanently disable the legacy arc MCP server.
- [ ] Fetch Claude docs → Markdown for read-only, citable reference snippets (cite specific lines; *derive* where valuable once the shape is clear). Sample sources under **References**.
- [ ] AI-driven ranking/prioritization of findings — what to tackle first. *(`suggest` is a first cut; `audit` ships for rule violations.)*
- [ ] A "lexicon" — canonical project terms + casing that linting enforces (would auto-catch drift like Claude Code / Codex / arclite casing). Likely lower priority.
- [ ] Fully review arc's codebase + feature set; clearly identify what made sense vs. what was sub-optimal/unnecessary.
- [ ] A **rules** system (see Open Questions).

## Open Questions/Ideas

- **Rules — format & lifecycle.** v1 is intentionally minimal: a rule is a **Markdown file** — its **filename (stem) is the `id`** (single source, no drift), its **contents are the body** (what enters the AI's context). Frontmatter/attributes for *selective inclusion* (`kind`, `scope`, `tags`, …) get added only when something actually filters on them — not before. Open: rename-stability of filename-ids; whether prompts/todos share the same format.
- **Rules — extraction & application.** Point arclite at a repo (e.g. `streamline`) to *extract* rules; aggregate them; configurably include some/all in any AI run (targeted or passive). The edition-2024 false positive from an early `suggest` run is a case in point — a version rule, or a "only flag violations of the provided rules" mode, would change the outcome *traceably*.
- **Gating / hooks for any command (configurable, cost-visible).** `--changed` already ships as a *shared* option — it scopes **any** synthesis command's context to git-changed files (staged/unstaged/untracked), not just `audit`. The open part is the passive side: wiring a command into git hooks (a commit gate that warns or blocks) so users/agents benefit without remembering to run it — again **general to any command, not special to `audit`**. Must be opt-in and **loud about cost and on/off status** — passive per-commit AI spend is precisely arc's failure mode; a hook that hides its cost would repeat it.
- **IDE & linter integration** — what would integrating with IDEs and linters mean/imply? (To be explored.)
- **Auto-context vs `--include` on real repos.** The default synthesis context (scan + root `README` + root-level manifests) is thin on real repos — manifests nest in subprojects (IDA, quant) and root READMEs are often stubs — so `--include` is needed for a substantive run today. Open: should auto-context pull the *detected* subdir manifests, or search wider for docs, vs. keeping a light default + explicit `--include`? (Surfaced exercising arclite on IDA/quant; a ~$1 capped review of quant's R core produced specific, code-grounded findings.)
- **Prompts as files?** Command prompts are inline in code today. Externalizing them — as **Markdown**, the same substrate as rules ("shared substrates"), rather than JSON/JSONL (which suit structured data, not prose) — would make them tunable without a rebuild and consistent with rules. Logical and on-thesis, but not urgent (inline works); do it when a prompt needs tuning without recompiling.
- **Agent-agnostic?** (e.g. Claude Code + Codex + any)
- **Dashboard?**
- **Distribution / install** — `cargo install`, prebuilt per-OS binaries from CI, `cargo-binstall`, Homebrew/Scoop?
- **CI** — build/test/release across Windows, macOS, Linux. *(GitHub Pipelines for Linux is in place; macOS/Windows would need runners.)*
- **LLM "synthesis" step** — Claude via CLI (`claude -p`), an SDK, or provider-agnostic?
- **Reclaim the `arc` name?** Eventually rename this repo (here + on GitHub) and take the `arc` CLI command/alias, superseding the legacy arc others may still have installed (this machine, Tom's, …). As the successor, arclite inheriting the name fits — but tentative; depends, not a directive.

## Related Repositories

- <https://github.com/nikganderson/arc/src/>
- <https://github.com/nikganderson/ida/src/>
- <https://github.com/nikganderson/quant/src/>

## References

Claude Code docs arclite leverages or draws on (cite specific behavior; *derive* where valuable — see the Roadmap item):

- <https://code.claude.com/docs/en/headless> — print/headless mode (`claude -p`): how arclite invokes the CLI.
- <https://code.claude.com/docs/en/cli-reference> — flags arclite passes (`--output-format json`, `--strict-mcp-config`, `--model`, `--allowedTools`, `--add-dir`).
- <https://code.claude.com/docs/en/memory> — CLAUDE.md + auto-memory, and the `CLAUDE_CODE_DISABLE_*` env vars arclite sets to isolate the synthesis.
- <https://code.claude.com/docs/en/settings> — settings layers/precedence.
- <https://code.claude.com/docs/en/permissions> — the tool-permission model behind `--allowedTools`.

## Other/Notes/Considerations

- Consider turning some sections into tables for readability and to avoid repetition.
