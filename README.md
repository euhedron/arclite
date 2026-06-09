# arclite

> *Nothing is canon. Everything can evolve.*
>
> **NOTE**: An experimental project under rapid development — treat any part of any file, and any architectural/formatting/structural decision, as in-progress, not settled (expect unfinished, abandoned, or sub-optimal thoughts).

## Overview

arclite is an **agent-first, cross-platform CLI for cross-repo code intelligence and auditing**. It gathers facts about a repository **deterministically**, and — only where genuine judgment is needed — applies **AI (via the Claude Code CLI)**. Every use is cost-transparent, configurable, and observable (see [Principles](#principles)). The aim: unlock analysis/auditing that doesn't already exist, while spending *sensibly*.

## Getting started

**Prerequisites:** a Rust toolchain (`cargo`; Rust ≥ 1.88, for let-chains — edition 2024 alone needs 1.85); the Claude Code CLI (`claude` on `PATH`) for the AI commands; and `git` (used by `--changed` and `arc init --hook`). `arc doctor` checks all three.

**Build / install:**

```sh
git clone https://github.com/nikganderson/arclite.git && cd arclite
cargo install --path .        # installs the `arc` command; or `cargo build --release` → target/release/arc
```

**Use** — the AI commands take a repo path (default `.`):

```sh
arc                                  # no args → help (the binary is `arc`; the project is arclite)
arc inspect <repo>                   # report structured facts about a repo
arc status                           # runs currently in flight
arc rules                            # the rules in play (ruleset, sources, provenance)
arc init    <repo>                   # add --hook for an opt-in pre-push gate
arc summarize <repo>                 # runs on the default context (scan + README + manifests)
arc suggest <repo> --include src     # --include adds files/dirs to the context
arc audit   <repo> --ruleset <id>    # --ruleset selects the rules (else the configured default)
arc critique <repo> --include src    # --dry-run previews any run's prompt + cost
arc extract <repo> --include src
```

Shared options on every AI command select rules (`--ruleset`/`--rules`), shape the context (`--include`, `--changed`, `--max-file-chars`), choose the model and tools (`--model`, `--allow-tool`, `--ambient-memory`), and control output (`--structured`, `--fail-on-findings` (**gate**), `--ranked`, `--runs`, `--output`, `--dry-run`, `--json`) — run `arc <command> --help` for the authoritative, always-current descriptions. Every run echoes the exact parameters it used — model, tools, memory isolation, the full **context manifest**, the active `.arc/settings.json` layers, and where the run was logged — alongside real token usage + cost (see [Principles](#principles)).

**Configuration** lives in `.arc/settings.json`, layered user (`~/.arc/`) then project (`<repo>/.arc/`): set defaults (model, ruleset, logging) and define **rulesets** — named compositions of *sources* (directories or files of Markdown rules, including shared pools). Project layers over user; `--ruleset`/`--rules` override per run. arclite's own rules live in `.arc/rules/` (its `self` ruleset, the configured default).

**Logging** — every *real* AI run appends a one-line JSON record — the run's parameters (command, repo, model, memory, context sources, and gate outcome) plus its ground-truth tokens + cost — to `~/.arc/logs/runs.jsonl`: a durable trace that outlives the terminal and is the substrate for "is the spend earning its keep" metrics. On by default; `arc doctor` shows the path and run count; `defaults.logging = false` turns it off; dry runs are never logged (no spend, nothing to record).

**Gating on push** (opt-in) — arclite's own tracked `hooks/pre-push` runs `critique` and `suggest` (non-blocking advisories) concurrently with the `arc audit --include src --fail-on-findings` gate, blocking the push on any violation. (`--include src` scopes it to arclite's own source; `arc init --hook` scaffolds the generic gate for any repo to adapt.) Enable for a clone with `git config core.hooksPath hooks`; skip one push with `ARC_GATE=0 git push`; disable by unsetting `core.hooksPath`. It spends real AI tokens per push (it announces this and prints the cost) — deliberately pre-*push*, not pre-commit, and opt-in, because passive per-commit AI spend is arc's failure mode.

## Background & motivation

arclite is a fresh, stripped-down **successor to** the legacy `arc` project — treated only as a *cautionary tale* and a *catalog of ideas to evaluate on merit*; nothing carries over by default. It is **not** meant to replace Claude Code or other tools (it can't keep up); the goal is to unlock new, efficient analysis/auditing workflows that don't already exist — *without* requiring users to understand its internals (arc was not user-friendly).

Two arc failure modes arclite must avoid:

- **Unsustainable AI spend** — arc's automations consumed usage at an unsustainable, opaque rate. arclite makes every AI use configurable, observable, and balanced.
- **Complexity outgrowing comprehension** — arc became too complex for new developers to understand or benefit from. arclite must stay intuitive as it grows.

Worth carrying over (on merit): self-derived/generated docs — users/agents shouldn't have to hand-maintain them.

## Specification

- **Platforms**: targets Windows, macOS, and Linux (built in Rust; ships as a single self-contained binary per platform); CI coverage is still being filled in (see [Open questions](#open-questions)).
- **CLI**: should be able to do and see anything, via flags.
- **Scope**: multi / any / cross repo — point it at any repository.
- **Rules as composable levers**: rules (coding standards, anti-patterns, principles) are reusable *levers* — not just prompts or memory — extracted from repos, curated, and composed into named **rulesets** that any command applies (their sources and scoping are covered under **Configuration**).
- **Auditing & linting**: check a repo against selected rules and surface drift/violations — on demand (a gate) or passively (e.g. commit hooks).
- **Discovery**: integrate with existing agent memory/config (Claude Code, Codex, …) — content storage and structure compatibility.
## Principles

The philosophy that defines arclite. (The *code's* own engineering standards — DRY, no hardcoding, single-source — live in `.arc/rules/` and are enforced via `audit`, not restated here.)

- **Agent-first, human-accessible** — usable by both agents and people.
- **Leverage, don't replace** existing, ever-evolving agent tools (e.g. the Claude Code CLI).
- **The CLI is the composition surface** — hooks, CI, and agents compose `arc` commands directly; don't re-encode invocations as a parallel config language that only mirrors argv and rots against it.
- **Maximally transparent, observable, and honest.**
- **Deterministic until synthesis** — gather/compute deterministically; reserve AI for the judgment step.
- **Sensible, observable, configurable AI spend** — no *arbitrary* defaults (the model default is configurable down for cost); preview before spending (`--dry-run`); report every run parameter alongside real token usage + cost; balance context utilization against value.
- **Trace, resolve, evolve** — unexpected/sub-par results are signal: make them traceable, diagnose, then improve the system — including the rules and prompts themselves, which are validated and sharpened through exercise, not assumed correct.
- **Adversarial, self-accountable** — build in self-checking (arclite is exercised on itself); a gate turns that into accountability — change proceeds only once the system is balanced, with no outstanding violations. The gate tests the **rules** as much as the code: resolve a finding by fixing the code, or — when the finding itself is off — by sharpening the rule.
- **Never done — balance is a floor, not a finish.** A clean audit isn't a win to rest on; it's the balanced state that lets the next change proceed. Commands and rules are levers to fire and tune to keep the loop running; when a repo stops yielding signal, point arclite at another to surface its own weaknesses.
- **Leverage derivation/transclusion.**

## Roadmap

Open and unsettled — not a plan, an ordering, or a commitment; it evolves (items get added, dropped, or reshaped as signals warrant).

- [ ] Aggregate extracted **rules** across repos and dedup them into shared pools (`extract` produces per-repo candidates today; the cross-repo merge is the open part).
- [ ] Aggregate per-run logs into metrics — across runs, repos, and (eventually) a team (command/gate frequency, audit pass-rate over time, cost trends) to see whether the rules are earning their keep. Per-run logging to `~/.arc/logs/runs.jsonl` ships; the cross-run/cross-repo/team rollup is the open part.
- [ ] **Multi-run strategies** — `--runs N` ships: run a command N times concurrently and union the deduped `results`. Open: a secondary-agent combine that dedupes/synthesizes semantically (and buckets by consensus, for ranking); sequential pass-forward (each run sees prior findings); and fanning the same union across *different* commands (e.g. a concurrent pre-push gate).
- [ ] `arc status` lists in-flight runs (ships) — a per-pid registry written on start and cleared on exit, the in-flight complement to the completed-run log. Open: pruning entries a hard-killed process leaves behind (a cross-platform liveness check); clean/error/unwind exits already clear themselves.
- [ ] Search across one or more repos.
- [ ] A "lexicon" — canonical project terms + casing that linting enforces (to auto-catch casing/naming drift in product and repo names).
- [ ] Fetch Claude docs → Markdown for citable reference snippets (cite specific lines; *derive* where valuable). Sources under **References**.
- [ ] Fully review arc's codebase + feature set; identify what made sense vs. what was sub-optimal.

## Open questions

- **Rules — format & lifecycle.** The rule format (a **Markdown file**; filename stem = `id`) and ruleset composition ship (see Configuration). Open: frontmatter for *selective inclusion* (`kind`, `scope`, `tags`), added only when something filters on it; rename-stability of filename-ids; whether prompts/todos share the format.
- **Rules — sharing & evolution.** `extract` mines candidates; curate them into a ruleset; `--ruleset` composes them into any run; `arc rules` shows the rules in play (ruleset, sources, per-rule provenance) (ships). Open: cross-repo aggregation + dedup of rules that recur in 2+ places (promote to a shared pool) and provenance-driven merge. (The edition-2024 false positive from an early `suggest` run is the kind of thing a sharpened rule resolves *traceably*.)
- **Gating / hooks for any command (cost-visible).** The **gate** ships: `--fail-on-findings` makes any command with a `results` list exit non-zero (code 2) when it's non-empty, now wired into an opt-in, cost-loud git **pre-push** hook. Still open: **Claude Code hook events** invoking `arc` (complementary to git hooks, not a replacement), and whether a pre-commit variant ever earns its keep.- **Command-kit identity.** The commands are one shared substrate differentiated only by prompt (`suggest` surfaces what's worth attention, `critique` finds defects, `audit` checks rules, `summarize` describes, `extract` mines rules). Watch for overlap/necessity/distinctness as the kit grows; let use — not speculation — justify each verb.
- **Auto-context depth.** The default context includes the *detected* manifests (root or nested) + the root README. Open: search wider for docs vs. keeping a light default + explicit `--include`.
- **Prompts as files?** Command prompts are inline in code today. Externalizing them as **Markdown** (the same substrate as rules) would make them tunable without a rebuild — do it when a prompt needs tuning without recompiling.
- **Reclaim the `arc` name.** The binary is *already* `arc` (it shadows any legacy `arc` at install time); the open part is renaming the repo (here + on GitHub) and formally superseding legacy arc.
- **Agent-agnostic?** (Claude Code + Codex + any.) **Distribution / install?** (`cargo install`, prebuilt per-OS binaries, `cargo-binstall`, Homebrew/Scoop.) **CI** across all three OSes (Linux is in place; macOS/Windows need runners). **Dashboard / IDE / linter integration?**

## Related repositories

- <https://github.com/nikganderson/arc/src/> — legacy arc (reference-only).
- <https://github.com/nikganderson/ida/src/> — IDA, a live repo arclite is exercised against.
- <https://github.com/nikganderson/quant/src/> — quant, likewise.

## References

Claude Code docs arclite leverages or draws on (cite specific behavior; *derive* where valuable — see the Roadmap item):

- <https://code.claude.com/docs/en/headless> — print/headless mode (`claude -p`): how arclite invokes the CLI.
- <https://code.claude.com/docs/en/cli-reference> — flags arclite passes (`--output-format json`, `--json-schema`, `--strict-mcp-config`, `--model`, `--allowedTools`, `--add-dir`, and `--max-budget-usd` as a prospective hard cost cap).
- <https://code.claude.com/docs/en/agent-sdk/structured-outputs> — `--json-schema` → a validated `structured_output` field: how arclite gets *typed* verdicts/findings (gating, ranking) instead of parsing prose.
- <https://code.claude.com/docs/en/memory> — CLAUDE.md + auto-memory, and the `CLAUDE_CODE_DISABLE_*` env vars arclite sets to isolate the synthesis.
- <https://code.claude.com/docs/en/hooks> — Claude Code's hook events: an agent-loop surface a hook can use to invoke `arc` (complementary to git hooks; arclite stays a citizen of existing hook systems rather than replacing them).
- <https://code.claude.com/docs/en/settings> — settings layers/precedence.
- <https://code.claude.com/docs/en/permissions> — the tool-permission model behind `--allowedTools`.
