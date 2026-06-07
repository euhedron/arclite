# arclite

> *Nothing is canon. Everything can evolve.*
>
> **NOTE**: This is an experimental project; development is rapid and every aspect/feature should be considered in-progress. Don't treat any part of any file — or any architectural, formatting, or structural decision — as settled (anticipate unfinished, abandoned, or sub-optimal thoughts, systems, and descriptions). Existing state is in-progress thinking, not settled decisions.

## Overview

arclite is an **agent-first, cross-platform CLI for cross-repo code intelligence and auditing**. It gathers facts about a repository **deterministically**, and — only where genuine judgment is needed — applies **AI (via the Claude Code CLI)**. Every AI use is cost-transparent, configurable, and observable (see [Principles](#principles)). The aim: unlock analysis/auditing that doesn't already exist, while spending AI *sensibly*.

| Command | What it does | AI |
|---|---|:--:|
| `doctor` | Report runtime, environment, and available tooling. | — |
| `inspect` | Walk any repo and emit structured facts (languages, layout, manifests, git state). | — |
| `summarize` | A brief assessment of a repo from its facts. | ✓ |
| `suggest` | A prioritized list of what's worth attention. | ✓ |
| `critique` | Quality defects — redundancy, staleness, gaps — each with a concrete fix. | ✓ |
| `extract` | Reusable rules (standards, anti-patterns, principles) to curate. | ✓ |
| `audit` | Check a repo against selected rules, reporting only concrete violations. | ✓ |

## Getting started

**Prerequisites:** a Rust toolchain (`cargo`; Rust ≥ 1.85, for edition 2024); and, for the AI commands, the Claude Code CLI installed and authenticated (`claude` on `PATH`). `arc doctor` verifies both.

**Build / install:**

```sh
git clone https://github.com/nikganderson/arclite.git && cd arclite
cargo install --path .        # installs the `arc` command; or `cargo build --release` → target/release/arc
```

**Use** — the deterministic commands (`doctor`, `inspect`) cost nothing; the AI commands take a repo path (default `.`):

```sh
arc                                  # self-documenting help (the binary is `arc`; the project is arclite)
arc inspect <repo>                   # deterministic facts, no AI (free)
arc summarize <repo>                 # brief AI assessment
arc suggest <repo> --include src     # prioritized review — preview with --dry-run ($0) first
arc critique <repo> --include src    # find imperfections + concrete fixes
arc audit   <repo> --ruleset <id>    # flag violations of a ruleset (or the repo's configured default)
arc extract <repo> --include src     # propose reusable rules to curate into a ruleset
```

Shared options on every AI command: `--ruleset <id>` (apply a named ruleset from settings) or `--rules <dir>` (an ad-hoc rule directory); `--structured` (emit a typed, schema-validated object instead of prose where the command defines one — e.g. `audit` violations, `suggest` a ranked list); `--model` (default `opus`; configure *down* for cost); `--include <path>` (add files/dirs to context); `--changed` (scope context to git-changed files — staged/unstaged/untracked); `--max-file-chars` (cap large files); `--output <dir>` (also save the result as a self-describing Markdown doc); `--ambient-memory` (load your `CLAUDE.md` instead of isolating); `--dry-run` (zero-cost preview); `--json`. Every run echoes the exact parameters it used and its token usage + cost — see [Principles](#principles).

**Configuration** lives in `.arc/settings.json`, layered user (`~/.arc/`) then project (`<repo>/.arc/`): set defaults (model, ruleset) and define **rulesets** — named compositions of *sources* (directories or files of Markdown rules, including shared pools). Project layers over user; `--ruleset`/`--rules` override per run. arclite's own rules live in `.arc/rules/` (its `self` ruleset, the configured default).

## Background & motivation

arclite is a fresh, stripped-down **successor to** the legacy `arc` project — treated only as a *cautionary tale* and a *catalog of ideas to evaluate on merit*; nothing carries over by default. It is **not** meant to replace Claude Code or other tools (it can't keep up); the goal is to unlock new, efficient analysis/auditing workflows that don't already exist — *without* requiring users to understand its internals (arc was not user-friendly).

Two arc failure modes arclite must avoid:

- **Unsustainable AI spend** — arc's automations consumed usage at an unsustainable, opaque rate. arclite makes every AI use configurable, observable, and balanced.
- **Complexity outgrowing comprehension** — arc became too complex for new developers to understand or benefit from. arclite must stay intuitive as it grows.

Worth carrying over (on merit): self-derived/generated docs — users/agents shouldn't have to hand-maintain them.

## Specification

- **Platforms**: targets Windows, macOS, and Linux (built in Rust; ships as a single self-contained binary per platform). Linux is CI-built today; macOS/Windows runners are pending (see [Open questions](#open-questions)).
- **CLI**: should be able to do and see anything, via flags.
- **Scope**: multi / any / cross repo — point it at any repository.
- **Rules as composable levers**: rules (coding standards, anti-patterns, principles) are reusable *levers* — not just prompts or memory. They're extracted from repos, curated, and composed into named **rulesets** that any command applies; a ruleset's sources span scopes — your own (`~/.arc/`), a project's (`<repo>/.arc/`), and shared pools — so broadly-useful rules get shared while the rest stay local.
- **Auditing & linting**: check a repo against selected rules and surface drift/violations — on demand (a gate) or passively (e.g. commit hooks). Configurable and cost-visible.
- **Discovery**: integrate with existing agent memory/config (Claude Code, Codex, …) — content storage and structure compatibility.
- **AI use**: deterministic until synthesis; AI is reserved for the judgment step, under the cost-transparency guarantees in [Principles](#principles).

## Principles

The philosophy that defines arclite. (The *code's* own engineering standards — DRY, no hardcoding, single-source — live in `rules/` and are enforced via `audit`, not restated here.)

- **Agent-first, human-accessible** — usable by both agents and people.
- **Leverage, don't replace** existing, ever-evolving agent tools (e.g. the Claude Code CLI).
- **Maximally transparent, observable, and honest.**
- **Deterministic until synthesis** — gather/compute deterministically; reserve AI for the judgment step.
- **Sensible, observable, configurable AI spend** — no *arbitrary* defaults (the model defaults to the *best*, configurable down for cost); preview at $0 (`--dry-run`); report every run parameter (model, tools, memory isolation, every context source) alongside real token usage + cost; balance context utilization against value.
- **Trace, resolve, evolve** — unexpected/sub-par results are signal: make them traceable, diagnose, then improve the system.
- **Adversarial** — build in self-checking (arclite is exercised on itself).
- **Leverage derivation/transclusion.**

## Roadmap

Open and unsettled — not a plan, an ordering, or a commitment; it evolves (items get added, dropped, or reshaped as signals warrant).

- [ ] Aggregate extracted **rules** across repos and dedup them into shared pools (`extract` produces per-repo candidates today; the cross-repo merge is the open part).
- [ ] Per-run logs + metrics (command/gate frequency, audit pass-rate over time, cost) — to see whether the rules are earning their keep.
- [ ] Gate a repo against rules *passively* (commit hooks) and rank/prioritize findings — `audit` flags violations on demand today; the passive + ranking parts are open.
- [ ] Search across one or more repos.
- [ ] A "lexicon" — canonical project terms + casing that linting enforces (to auto-catch casing/naming drift in product and repo names).
- [ ] Fetch Claude docs → Markdown for citable reference snippets (cite specific lines; *derive* where valuable). Sources under **References**.
- [ ] Fully review arc's codebase + feature set; identify what made sense vs. what was sub-optimal.

## Open questions

- **Rules — format & lifecycle.** A rule is a **Markdown file** (filename stem = `id`, contents = body). Rulesets compose *sources* (dirs/files of rules) and are defined in `.arc/settings.json`, layered user then project — that part ships. Open: frontmatter for *selective inclusion* (`kind`, `scope`, `tags`), added only when something filters on it; rename-stability of filename-ids; whether prompts/todos share the format.
- **Rules — sharing & evolution.** `extract` mines candidates; curate them into a ruleset; `--ruleset` composes them into any run (ships). Open: cross-repo aggregation + dedup of rules that recur in 2+ places (promote to a shared pool), provenance-driven merge, and visibility into the scope of rules in play. (The edition-2024 false positive from an early `suggest` run is the kind of thing a sharpened rule resolves *traceably*.)
- **Gating / hooks for any command (cost-visible).** `--changed` already ships as a shared option. The open part is the passive side: wiring a command into git hooks (a commit gate that warns or blocks) so users/agents benefit without remembering to run it — general to any command, not special to `audit`. Must be opt-in and **loud about cost and on/off status** — passive per-commit AI spend is precisely arc's failure mode.
- **Command-kit identity.** The commands are one shared substrate differentiated only by prompt (`suggest` prioritizes, `critique` finds defects, `audit` checks rules, `summarize` describes, `extract` mines rules). Watch for overlap/necessity/distinctness as the kit grows; let use — not speculation — justify each verb.
- **Auto-context depth.** The default context includes the *detected* manifests (root or nested) + the root README. Open: search wider for docs vs. keeping a light default + explicit `--include`.
- **Prompts as files?** Command prompts are inline in code today. Externalizing them as **Markdown** (the same substrate as rules) would make them tunable without a rebuild — do it when a prompt needs tuning without recompiling.
- **Reclaim the `arc` name.** The binary is *already* `arc` (it shadows any legacy `arc` at install time); the open part is renaming the repo (here + on GitHub) and formally superseding legacy arc.
- **Agent-agnostic?** (Claude Code + Codex + any.) **Distribution / install?** (`cargo install`, prebuilt per-OS binaries, `cargo-binstall`, Homebrew/Scoop.) **CI** across all three OSes (Linux is in place; macOS/Windows need runners). **Dashboard / IDE / linter integration?**

## Related repositories

- <https://github.com/nikganderson/arc/src/> — legacy arc (reference-only).
- <https://github.com/nikganderson/ida/src/>
- <https://github.com/nikganderson/quant/src/>

## References

Claude Code docs arclite leverages or draws on (cite specific behavior; *derive* where valuable — see the Roadmap item):

- <https://code.claude.com/docs/en/headless> — print/headless mode (`claude -p`): how arclite invokes the CLI.
- <https://code.claude.com/docs/en/cli-reference> — flags arclite passes (`--output-format json`, `--json-schema`, `--strict-mcp-config`, `--model`, `--allowedTools`, `--add-dir`, and `--max-budget-usd` as a prospective hard cost cap).
- <https://code.claude.com/docs/en/agent-sdk/structured-outputs> — `--json-schema` → a validated `structured_output` field: how arclite gets *typed* verdicts/findings (gating, ranking) instead of parsing prose.
- <https://code.claude.com/docs/en/memory> — CLAUDE.md + auto-memory, and the `CLAUDE_CODE_DISABLE_*` env vars arclite sets to isolate the synthesis.
- <https://code.claude.com/docs/en/hooks> — Claude Code's hook events: an agent-loop surface a hook can use to invoke `arc` (complementary to git hooks; arclite stays a citizen of existing hook systems rather than replacing them).
- <https://code.claude.com/docs/en/settings> — settings layers/precedence.
- <https://code.claude.com/docs/en/permissions> — the tool-permission model behind `--allowedTools`.
