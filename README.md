# arclite

> *Nothing is canon. Everything can evolve.*
>
> **NOTE**: An experimental project under rapid development — treat any part of any file, and any architectural/formatting/structural decision, as in-progress, not settled (expect unfinished, abandoned, or sub-optimal thoughts).

## Overview

arclite is an **agent-first, cross-platform CLI for cross-repo code intelligence and auditing**. It gathers facts about a repository **deterministically**, and — only where genuine judgment is needed — applies **AI** (via an agent CLI — Claude Code or Codex, selectable per run). Every use is cost-transparent, configurable, and observable (see [Principles](#principles)). The aim: unlock analysis/auditing that doesn't already exist, while spending *sensibly*.

## Getting started

**Prerequisites:** a Rust toolchain (`cargo`; Rust as new as the crate's `rust-version`, for let-chains); an agent CLI on `PATH` for the AI commands — the Claude Code CLI (`claude`, the default) or the Codex CLI (`codex`, via `--backend codex`); and `git` (used by `--changed`, `arc init --hook`, `arc update`, and to stamp each run's commit provenance); `curl` is needed by `arc update --apply` and the provider model listings (`arc models`, the TUI's model pickers). `arc doctor` checks for all of these.

**Install.** Two ways:

*Download & run* (no toolchain — for using arc on your own repos): grab the latest `arc-v<version>-<os>-<arch>` binary for your platform from the repo's **[Releases](https://github.com/nikganderson/arclite/releases)**, put it on your `PATH` as `arc` (`arc.exe` on Windows; `chmod +x` on Linux/macOS), then run `arc doctor`. No clone, no Rust. (Binaries are published by the release workflow on each version tag; until the first tagged release lands, build *From source* below.)

*From source* (to develop arclite):

```sh
git clone https://github.com/nikganderson/arclite.git && cd arclite
cargo install --path .        # installs the `arc` command; or `cargo build --release` → target/release/arc
```

**Use** — common commands (a `<repo>` argument defaults to `.`):

```sh
arc                                  # no args → help (the binary is `arc`; the project is arclite)
arc inspect <repo>
arc doctor
arc update
arc status
arc tui
arc log [<id>]
arc usage
arc rules
arc models
arc config set defaults.model <id>
arc init <repo> --hook
arc run summarize <repo>
arc run suggest   <repo> --include src
arc run audit     <repo> --ruleset <id>
arc run critique  <repo> --backend codex
arc run extract   <repo> --runs 3
arc run evolve    <repo> --ranked
arc run verify    <repo> --changed
arc promote <run-id>
arc retire  <verify-run-id>
```

Run `arc <command> --help` for what a command does and its full options — authoritative there, not restated here where the copy would rot. The AI commands share one option surface (select the rules, shape the context, choose the model/tools/backend, bound or preview the spend, control output and the **gate**). Every run echoes the exact parameters it used — model, tools, memory isolation, the full list of **context sources**, the active `.arc/settings.json` layers, and where the run was logged — alongside real token usage + cost (codex reports tokens only — no dollar cost).

**Configuration** lives in `.arc/settings.json`, layered user (`~/.arc/`) then project (`<repo>/.arc/`): set command defaults — by hand or with `arc config set` — and define **rulesets**, named compositions of *sources* (directories or files of Markdown rules, including shared pools). Project layers over user; `--ruleset`/`--rules` override per run. arclite's own rules are its `self` ruleset, the configured default. A root-level `disabled_rules` list switches individual rules off by id — filtered out of every run and report, always disclosed, never silent (`arc config set disabled_rules a,b`, or toggle with space in the TUI's rules view). Provider API keys (for `arc models` and the TUI's model pickers) are user-layer only and masked in every output; the standard env vars take precedence.

**Logging** — every *real* AI run appends its parameters and ground-truth tokens + cost as a one-line JSON record — a completed run, or one that errored mid-flight after spending (either backend), so a failed run's real cost is recorded rather than lost — and stores its full result alongside: a durable trace that outlives the terminal and the substrate for "is the spend earning its keep" metrics (`arc usage` rolls it up locally; `arc log` and `arc log <id>` browse and re-show runs without re-running them). On by default (`defaults.logging = false` turns it off); dry runs are never logged — no spend, nothing to record. `arc doctor` shows where.

**Gating on push** (opt-in) — a git **pre-push hook** runs arc commands as a gate: `arc run audit --fail-on-findings` exits non-zero on violations, blocking the push. arclite ships a tracked `.arc/hooks/pre-push` as a *starting point, not a canon* — read it to see the current setup, and edit it freely; a hook is configurable by definition (`arc init --hook` scaffolds a minimal one for any repo). A fresh scaffold gates against an empty starter ruleset, passing vacuously (and saying so) until rules are curated, so the hook can be wired up before the rules exist. Enable for a clone with `git config core.hooksPath .arc/hooks`; skip one push with `ARC_GATE=0 git push`; disable by unsetting `core.hooksPath`. It spends real AI tokens per push (announced, with the cost printed) — deliberately pre-*push*, not pre-commit, and opt-in, because passive per-commit AI spend is arc's failure mode.

## Background & motivation

arclite is a fresh, stripped-down **successor to** the legacy `arc` project — an early, premature seed of this same arc rather than a separate predecessor: its lessons (cautionary *and* generative) and ideas are carried forward on merit; nothing carries over by default. It is **not** meant to replace Claude Code or other tools (it can't keep up); the goal is to unlock new, efficient analysis/auditing workflows that don't already exist — *without* requiring users to understand its internals (arc was not user-friendly).

Two arc failure modes arclite must avoid:

- **Unsustainable AI spend** — arc's automations consumed usage at an unsustainable, opaque rate. arclite makes every AI use configurable, observable, and balanced.
- **Complexity outgrowing comprehension** — arc became too complex for new developers to understand or benefit from. arclite must stay intuitive as it grows.

Worth carrying over (on merit): self-derived/generated docs — users/agents shouldn't have to hand-maintain them.

## Specification

- **Platforms**: targets Windows, macOS, and Linux (built in Rust; ships as a single self-contained binary per platform).
- **CLI**: should be able to do and see anything, via flags.
- **Scope**: multi / any / cross repo — point it at any repository.
- **Rules as composable levers**: rules (coding standards, anti-patterns, principles) are reusable *levers* — not just prompts or memory — extracted from repos, curated, and composed into named **rulesets** that any command applies (their sources and scoping are covered under **Configuration**). A rule must capture a **general principle** — the larger intelligence, applicable across repos and contexts — never narrow wording aimed at one codebase's specific attribute, nor a rule adopted because it would catch something in the repo at hand; a clearly-stated principle needs no examples.
- **Auditing & linting**: check a repo against selected rules and surface drift/violations — on demand (a gate) or passively (a pre-push hook).
- **Discovery**: integrate with existing agent memory/config (Claude Code, Codex, …) — content storage and structure compatibility.
## Principles

The philosophy that defines arclite. (The *code's* own engineering standards — DRY, no hardcoding, single-source — live in `.arc/rules/` and are enforced via `audit`, not restated here.)

- **Agent-first, human-accessible** — usable by both agents and people.
- **Leverage, don't replace** existing, ever-evolving agent tools (e.g. the Claude Code CLI).
- **The CLI is the composition surface** — hooks, CI, and agents compose `arc` commands directly; don't re-encode invocations as a parallel config language that only mirrors argv and rots against it.
- **Maximally transparent, observable, and honest.**
- **Deterministic until synthesis** — gather/compute deterministically; reserve AI for the judgment step.
- **Sensible, observable, configurable AI spend** — no *arbitrary* defaults (the model default is configurable down for cost); preview before spending (`--dry-run`); hard-cap a run (`--max-budget-usd`, on backends with a native cap); report every run parameter alongside real token usage + cost; balance context utilization against value.
- **Every run is signal** — each AI run produces *unique* signal: either productive change, or a pointer to how the system (rules, prompts, context) should change to yield better signal next. Runs are therefore never cached or memoized to "save" spend — re-running *is* the value; sensible spend is spend that *earns* signal, not spend *minimized*.
- **Trace, resolve, evolve** — unexpected/sub-par results are signal: make them traceable, diagnose, then improve the system — including the rules and prompts themselves, which are validated and sharpened through exercise, not assumed correct.
- **Adversarial, self-accountable** — build in self-checking (arclite is exercised on itself); a gate turns that into accountability — change proceeds only once the system is balanced, with no outstanding violations. The gate tests the **rules** as much as the code: resolve a finding by fixing the code, or — when the finding itself is off — by sharpening the rule.
- **Never done — balance is a floor, not a finish.** A clean audit isn't a win to rest on; it's the balanced state that lets the next change proceed. Commands and rules are levers to fire and tune to keep the loop running; when a repo stops yielding signal, point arclite at another to surface its own weaknesses.
- **Leverage derivation/transclusion/resolution** — derive docs and content from the system rather than hand-maintaining parallel copies; reach for transclusion-style machinery only where mix-and-match flexibility genuinely earns it. Plain duplication needs no machinery — it's a defect: give each fact one home and point to it.

## Roadmap

Open and unsettled — not a plan, an ordering, or a commitment; it evolves (items get added, dropped, or reshaped as signals warrant). Two lists, so what has landed and what is open can't blur.

**Landed, with open edges:**

- [x] **Multi-run** — `--runs N` runs a command N times concurrently and unions the `results` (only byte-identical items collapse). The substance-merge that edge called for is now the `aggregate` verb (under **Open**). Open: sequential pass-forward (each run sees prior findings); fanning one union across *different* commands.
- [x] **Run logging + rollup** — mechanism under **Logging** above. Open: cross-repo and team aggregation; trends over time (audit pass-rate, cost curves) to see whether the rules earn their keep.
- [x] **`arc status` + live stats** — one marker file per in-flight run, streamed with live progress (output characters, turns, tool-calls) so `arc status` and the TUI footer track it live. A marker orphaned by a hard-killed process is pruned by a pid-liveness probe (Unix `kill -0`, Windows `tasklist`) and disclosed, never reported as active; an inconclusive probe keeps it (over-reporting one dead run beats hiding a live one). Open: richer per-run detail.
- [x] **Codex backend** — `--backend codex` drives `codex exec` behind one `Backend` trait, so usage spreads across subscriptions; runs are self-contained and isolated by default (AGENTS.md suppressed like CLAUDE.md; `--ambient-memory` opts it back in). A capability a backend can't honor is rejected before spend (codex has no native cost cap; `--allow-tool` isn't yet mapped to its MCP/sandbox model). Open: the `--allow-tool`→codex bridge.
- [x] **`arc tui`** — an interactive cockpit over the same commands: inline (preserves scrollback), render a pure function of state. Every deterministic command has a view (status/config/rules/log/usage/doctor); config editing and rule toggling write through the same path as `arc config set`; the launch gate previews a verb's `--dry-run` at zero spend, and shaping keys (`b`/`m`/`r` for backend/model/ruleset) re-run the preview so the shown command is exactly the one fired. Open: shaping the launch's *context* (include/exclude/changed); surfacing a failed launch inline; live output streaming; and the larger aim — findings and extracted rules as interactive structured items acted on in place.
- [x] **Release pipeline + `arc update`** — `.github/workflows/release.yml` builds per-OS binaries and uploads them to [Releases](https://github.com/nikganderson/arclite/releases) on a version tag; `arc update` checks the latest and `--apply` self-replaces (a private repo's assets take an optional `ARC_GITHUB_TOKEN`, kept off argv). Update integrity rests on HTTPS transport + the GitHub release's authenticity, not a bundled signature — a same-channel checksum shares the release's trust root, so real tamper-resistance would need an out-of-band signing key or build attestations. Open: `cargo-binstall`/Homebrew/Scoop; signed-release verification.
- [x] **Surface available models** — `arc models` (and the TUI's model pickers) list each backend's provider-reported models (Anthropic/OpenAI `/v1/models`), keyed by the standard env var or a masked, **user-layer-only** `api_keys.*` setting (the loader rejects a project-layer key, so a tracked settings.json can never hold a secret). Open: reconciling the compile-time model defaults against the live listing.
- [x] **Errored runs logged on both backends** — a run that fails mid-flight after spending records its real usage as a logged errored run, so the spend is traced rather than lost.

**Open:**

- [ ] **Aggregate rules/findings across repos** — `arc run aggregate --from <run-id> …` feeds prior logged runs' structured results into context and merges the items that are the *same in substance* (the AI judges sameness; each merged item carries the `sources` it drew from, so recurrence is counted, not model-asserted, and `covered_by` flags an active rule that already expresses it). Works over any structured runs — extract candidates, audit findings. Open: promoting merged rules to shared pools; a `--repos` convenience that runs the extracts before aggregating. Extract candidates banked against the curation bar: `distinguish-unknown-from-zero`, `snapshot-referenced-state-at-record-time`/`reconcile-cached-verdict-with-live-state`, `idempotent-reingest-via-stable-keys`, `stream-unbounded-external-data`, `resolve-packaged-assets-relative-to-module`, `revalidate-gate-before-irreversible-commit`, `aggregate-from-authoritative-ledger`, `curated-list-orders-not-filters`, `group-parameters-into-a-struct`, `impose-deterministic-order-on-unordered-sources`, `feed-child-stdin-without-deadlocking`, `scaffolding-must-be-idempotent-and-nondestructive`, `reload-canonical-state-after-write`; and `ai-inference-stays-advisory-never-authoritative` (central to the sibling systems but likely too architectural for a concrete code rule).
- [ ] **Promote findings into the repo** — the **extract → promote → verify → retire** lifecycle is whole and system-owned: `arc promote <run-id>` writes a run's structured findings into `.arc/findings/open/` (atomic names, commit-anchored, seeding an agent-facing ledger README on first use); `arc run verify` re-checks the open ledger against current code (`reproduces` | `resolved` | `indeterminate`, each grounded in a cited mechanism); `arc retire` moves the resolved ones into `resolved/`. `--findings` feeds the ledger into a run to hunt past what's known. Open: cross-scope curation (repo vs. `~/.arc`); same-issue supersession on re-promote; description-based entry naming (today it takes the finding's longest field, often an identifier slug).
- [ ] **Smooth onboarding & rule capture** — `arc init`, then extract → curate → audit, should run end to end without friction. Exercised on foreign repos (a .NET solution; the crux/commissure sibling products), which surfaced and closed real gaps (`--exclude`, `--no-scan`, a top-level `inspect` layout). Open: the broader friction of the whole loop without hand-holding.
- [ ] **Feed VCS-tracking truth into the synthesis context.** The context carries file *contents* but not their git-tracking state, so the model can call a gitignored file "committed." arclite is already git-aware (gitignore-aware walk, `--changed`); surfacing tracked-vs-ignored ground truth would let the deterministic layer constrain such claims rather than leave the model to infer them — *deterministic until synthesis*. (A related seam landed: a *deleted* path in `--changed` is excluded from context and disclosed, not mis-surfaced as "missing.")
- [ ] Search across one or more repos.
- [ ] A "lexicon" — canonical project terms + casing that linting enforces (to auto-catch casing/naming drift in product and repo names).
- [ ] Fetch Claude docs → Markdown for citable reference snippets (cite specific lines; *derive* where valuable). Sources under **References**.
- [ ] Fully review arc's codebase + feature set; identify what made sense vs. what was sub-optimal.

## Open questions

- **Rules — format & lifecycle.** A rule is a **Markdown file** (filename stem = `id`); rulesets compose sources (see **Configuration**). The ruleset audits *itself* for generality: a `rule-quality` ruleset of meta-rules under `.arc/meta-rules/` audits `.arc/rules` in the pre-push loop when rules change, so a curated rule can't land example-bound or tied to arclite's current state (it has caught real current-state bindings in arclite's own rules). Rules earn inclusion by recurrence across repos — `dispatch-through-one-registry`, `report-the-identity-that-ran`, and `read-structured-data-not-reparsed-prose` (the first arclite learned from the sibling products rather than itself); candidates still awaiting the bar are banked under the aggregation item above. Open: frontmatter for selective inclusion (`kind`/`scope`/`tags`), added only when something filters on it; rename-stable ids; applying the meta-audit recursively; set-level redundancy/overlap checks beyond the per-rule lens.
- **Gating / hooks for any command (cost-visible).** The gate (`--fail-on-findings`) and the pre-push hook ship (see **Gating on push**). Open: **Claude Code hook events** invoking `arc` (complementary to git hooks, not a replacement); whether a pre-commit variant earns its keep; and whether a `--runs N` union on the gate's audit (more findings per round, exercised once and it did surface a gap the single runs missed) or moving the non-blocking advisories to periodic/on-demand better balances cost against signal — a heavy push tends to converge one finding per round rather than exhausting in one pass.
- **Command-kit identity.** The verbs are one prompt-differentiated substrate under `arc run <verb>`, kept distinct from the deterministic top-level commands — `suggest` (what's worth attention), `critique` (defects), `audit` (rule violations), `verify` (re-checks recorded findings), `summarize` (describes), `extract` (mines rules), `evolve` (challenges the frame), `aggregate` (merges prior runs by substance). An `aggregate` of a `critique` + `suggest` run over arclite showed them **complementary** — converging only on the single most-salient issue, otherwise each surfacing its own, with `audit` silent where the code already meets the settled bar. Watch overlap/necessity/distinctness as the kit grows; let use, not speculation, justify each verb.
- **Auto-context depth.** The default context is the detected manifests + root README; `--no-scan` drops that scan baseline so a diff-scoped run's cost tracks the diff, not a whole-repo baseline. Open: whether to also pull the root `CLAUDE.md`/`AGENTS.md` as an explicit (reported) source — arc's **Discovery** goal argues for it, distinct from the agent's isolated ambient load; whether a lean run should drop the README too; how wide to search for prose docs vs. a light default + explicit `--include`.
- **Prompts as files?** Command prompts are inline in code today. Externalizing them as **Markdown** (the same substrate as rules) would make them tunable without a rebuild — do it when a prompt needs tuning without recompiling.
- **Structured output vs. tool calls.** A command's typed result can come as a final **structured-output** artifact (today's `--json-schema`/`--output-schema` → `results`+`note`) or the model **calling a tool** with structured *input* — near-equivalent in expressiveness, differing in lifecycle (one parse point vs. per-item events + partial salvage), not a settled quality claim. arclite uses structured output; tool use is a lever for when a use-case earns it. Open edge surfaced by exercise: an `audit` once emitted an `extract`-shaped item that slipped the declared schema and gated a push — arclite should re-validate each structured result against the verb's shape before acting on it, trusting the channel less.
- **Reclaim the `arc` name.** The binary is *already* `arc` (it shadows any legacy `arc` at install time); the open part is formally superseding legacy arc.
- **Agent-agnostic?** (Claude Code + Codex + any.) **Distribution** — per-OS binaries on GitHub Releases (via the release workflow) and `arc update` (check + `--apply` self-replace) ship now; the still-open edges live in the Roadmap's release-pipeline item, not restated here. **CI** — GitHub Actions runs build/test/lint on Linux + macOS, with a tag-triggered release build per platform. **Dashboard / IDE / linter integration?**

## References

Claude Code docs arclite leverages or draws on (cite specific behavior; *derive* where valuable — see the Roadmap item):

- <https://code.claude.com/docs/en/headless> — print/headless mode (`claude -p`): how arclite invokes the CLI.
- <https://code.claude.com/docs/en/cli-reference> — flags arclite passes (`-p`, `--output-format stream-json --include-partial-messages --verbose`, `--json-schema`, `--strict-mcp-config`, `--model`, `--allowedTools`, `--add-dir`, and `--max-budget-usd`, the hard per-run cost cap).
- <https://code.claude.com/docs/en/agent-sdk/structured-outputs> — `--json-schema` → a validated `structured_output` field: how arclite gets *typed* verdicts/findings (gating, ranking) instead of parsing prose.
- <https://code.claude.com/docs/en/memory> — CLAUDE.md + auto-memory, and the `CLAUDE_CODE_DISABLE_*` env vars arclite sets to isolate the synthesis.
- <https://code.claude.com/docs/en/hooks> — Claude Code's hook events: the agent-loop surface for invoking `arc` (the open question under **Gating / hooks**).
- <https://code.claude.com/docs/en/settings> — settings layers/precedence.
- <https://code.claude.com/docs/en/permissions> — the tool-permission model behind `--allowedTools`.
- <https://platform.claude.com/docs/en/build-with-claude/streaming> — the Messages API streaming event flow (`message_start` → `content_block_delta`s → one `message_delta`): the exact token count lands only at message end, so `arc status` streams live output *characters* and reports exact tokens at completion.
- <https://code.claude.com/docs/en/interactive-mode> — keyboard shortcuts, input modes, command history, the status-area task list, footer status indicators: precedent the `arc tui` draws on.
- <https://code.claude.com/docs/en/terminal-config> — terminal behaviors a TUI must respect (multiline-input keys, notifications, flicker-free fullscreen rendering, theming); behaviors the `arc tui` respects.

Codex CLI docs (codex is a synthesis backend — `--backend codex`; also open-source Rust, a reference for the `arc tui`):

- <https://developers.openai.com/codex/noninteractive> — `codex exec`: the headless entry arclite drives (`--json` events, `--output-schema`, `-o`, prompt on stdin).
- <https://developers.openai.com/codex/cli/reference> — CLI flags (top-level vs `exec`): `-m`, `-s/--sandbox`, `-c`, `-C/--cd`, `--output-schema`, `-o`, `--skip-git-repo-check`, `--ignore-user-config`, `--ignore-rules`.
- <https://developers.openai.com/codex/cli/features> — the feature surface (exec, MCP, images, resume, model selection).
- <https://developers.openai.com/codex/cli/slash-commands> — interactive slash commands (TUI precedent; several have config/flag equivalents).
- <https://developers.openai.com/codex/config-basic> — config basics (`~/.codex/config.toml`, model, reasoning, sandbox/approval).
- <https://developers.openai.com/codex/config-reference> — full config keys: `model_reasoning_effort` (`minimal|low|medium|high|xhigh`), `project_doc_max_bytes` (the AGENTS.md control), `sandbox_mode`, `approval_policy`, `shell_environment_policy`.
- <https://developers.openai.com/codex/config-advanced> — profiles, per-project config, AGENTS.md/instruction control, MCP.
- <https://developers.openai.com/codex/guides/agents-md> - `AGENTS.md` discovery/layering: global vs project scope, override files, fallback filenames, merge order, and `project_doc_max_bytes`.
- <https://developers.openai.com/codex/config-sample> — a worked sample `config.toml`.
- <https://developers.openai.com/codex/environment-variables> — env vars: `CODEX_API_KEY` (exec auth), `CODEX_HOME`, TLS/cert.
- <https://developers.openai.com/codex/permissions> — the permission model (presets, `default_permissions`, network) — note it does *not* compose with `sandbox_mode`.
- <https://developers.openai.com/codex/agent-approvals-security> — sandbox modes × approval policies; the locked-down non-interactive combo (`sandbox read-only` + `approval_policy=never`).
- <https://developers.openai.com/codex/mcp> — MCP servers/tools (`[mcp_servers]`): codex's tool model (the `--allow-tool`→codex bridge target).
- <https://github.com/openai/codex> — codex source (open-source Rust): the backend's ground truth, and a worked example for the `arc tui`.
