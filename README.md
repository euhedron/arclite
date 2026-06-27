# arclite

> *Nothing is canon. Everything can evolve.*
>
> **NOTE**: An experimental project under rapid development — treat any part of any file, and any architectural/formatting/structural decision, as in-progress, not settled (expect unfinished, abandoned, or sub-optimal thoughts).

## Overview

arclite is an **agent-first, cross-platform CLI for cross-repo code intelligence and auditing**. It gathers facts about a repository **deterministically**, and — only where genuine judgment is needed — applies **AI** (via an agent CLI — Claude Code or Codex, selectable per run). Every use is cost-transparent, configurable, and observable (see [Principles](#principles)). The aim: unlock analysis/auditing that doesn't already exist, while spending *sensibly*.

## Getting started

**Prerequisites:** a Rust toolchain (`cargo`; Rust as new as the crate's `rust-version`, for let-chains); an agent CLI on `PATH` for the AI commands — the Claude Code CLI (`claude`, the default) or the Codex CLI (`codex`, via `--backend codex`); and `git` (used by `--changed` and `arc init --hook`). `arc doctor` checks for all of these.

**Install.** Two ways:

*Download & run* (no toolchain — for using arc on your own repos): grab the latest `arc-<version>-<os>` from the repo's **[Downloads](https://github.com/nikganderson/arclite/downloads)** (you'll be signed into GitHub), put it on your `PATH` as `arc` (`arc.exe` on Windows; `chmod +x` on Linux/macOS), then run `arc doctor`. No clone, no Rust.

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
arc status
arc tui                              # the interactive cockpit (human-facing front-end)
arc log [<id>]
arc usage
arc rules
arc config set defaults.model <id>
arc init <repo> --hook
arc run summarize <repo>
arc run suggest   <repo> --include src
arc run audit     <repo> --ruleset <id>
arc run critique  <repo> --backend codex
arc run extract   <repo> --runs 3
arc run evolve    <repo> --ranked
```

Run `arc <command> --help` for what a command does and its full options — authoritative there, not restated here where the copy would rot. The AI commands share one option surface (select the rules, shape the context, choose the model/tools/backend, bound or preview the spend, control output and the **gate**). Every run echoes the exact parameters it used — model, tools, memory isolation, the full list of **context sources**, the active `.arc/settings.json` layers, and where the run was logged — alongside real token usage + cost (codex reports tokens only — no dollar cost).

**Configuration** lives in `.arc/settings.json`, layered user (`~/.arc/`) then project (`<repo>/.arc/`): set command defaults — by hand or with `arc config set` — and define **rulesets**, named compositions of *sources* (directories or files of Markdown rules, including shared pools). Project layers over user; `--ruleset`/`--rules` override per run. arclite's own rules are its `self` ruleset, the configured default.

**Logging** — every *real* AI run appends its parameters and ground-truth tokens + cost as a one-line JSON record — a completed run, or (on claude) one that errored mid-flight after spending, so a failed run's real cost is recorded rather than lost — and stores its full result alongside: a durable trace that outlives the terminal and the substrate for "is the spend earning its keep" metrics (`arc usage` rolls it up locally; `arc log` and `arc log <id>` browse and re-show runs without re-running them). On by default (`defaults.logging = false` turns it off); dry runs are never logged — no spend, nothing to record. `arc doctor` shows where.

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
- **Sensible, observable, configurable AI spend** — no *arbitrary* defaults (the model default is configurable down for cost); preview before spending (`--dry-run`); hard-cap any run (`--max-budget-usd`); report every run parameter alongside real token usage + cost; balance context utilization against value.
- **Every run is signal** — each AI run produces *unique* signal: either productive change, or a pointer to how the system (rules, prompts, context) should change to yield better signal next. Runs are therefore never cached or memoized to "save" spend — re-running *is* the value; sensible spend is spend that *earns* signal, not spend *minimized*.
- **Trace, resolve, evolve** — unexpected/sub-par results are signal: make them traceable, diagnose, then improve the system — including the rules and prompts themselves, which are validated and sharpened through exercise, not assumed correct.
- **Adversarial, self-accountable** — build in self-checking (arclite is exercised on itself); a gate turns that into accountability — change proceeds only once the system is balanced, with no outstanding violations. The gate tests the **rules** as much as the code: resolve a finding by fixing the code, or — when the finding itself is off — by sharpening the rule.
- **Never done — balance is a floor, not a finish.** A clean audit isn't a win to rest on; it's the balanced state that lets the next change proceed. Commands and rules are levers to fire and tune to keep the loop running; when a repo stops yielding signal, point arclite at another to surface its own weaknesses.
- **Leverage derivation/transclusion/resolution** — derive docs and content from the system rather than hand-maintaining parallel copies; reach for transclusion-style machinery only where mix-and-match flexibility genuinely earns it. Plain duplication needs no machinery — it's a defect: give each fact one home and point to it.

## Roadmap

Open and unsettled — not a plan, an ordering, or a commitment; it evolves (items get added, dropped, or reshaped as signals warrant). Two lists, so what has landed and what is open can't blur.

**Landed, with open edges:**

- [x] **Multi-run** — `--runs N`: run a command N times concurrently and union the `results` (only byte-identical items collapse — independent runs rarely emit the same prose verbatim, so real merging stays open). Open: a secondary-agent combine that judges which findings are the same in substance — and buckets by consensus, for ranking; sequential pass-forward (each run sees prior findings); and fanning the same union across *different* commands (e.g. a concurrent pre-push gate).
- [x] **Run logging + local rollup** — every completed run appends to `~/.arc/logs/runs.jsonl` (ground-truth usage + cost and the run's parameters), and `arc usage` rolls it up: runs, blocks, tokens, and cost by hour/day/week/total, plus per-command totals. Open: cross-repo and (eventually) team aggregation; trends over time (audit pass-rate, cost curves) to see whether the rules are earning their keep.
- [x] **`arc status`** — in-flight runs: one marker file per run, written on start and cleared on exit (a `--runs N` fan-out is N independent markers). Open: pruning entries a hard-killed process leaves behind (a cross-platform liveness check); clean/error/unwind exits already clear themselves.
- [x] **Live run stats** — the synthesis layer streams the CLI's events (`--output-format stream-json --include-partial-messages`) and updates each run's marker as they arrive, so `arc status` shows live progress: output **characters** (the continuous signal; exact tokens land only at completion — the streaming Reference explains why), plus turns and tool-calls at each boundary. Open: richer per-run detail.
- [x] **Codex backend** — `--backend codex` (+ `defaults.backend`) drives `codex exec` behind one `Backend` trait, so usage can spread across subscriptions. Runs are self-contained (ignore the user's codex config; model/reasoning/sandbox/approval set explicitly) and isolated by default (AGENTS.md suppressed, mirroring claude's CLAUDE.md; `--ambient-memory` opts it back in — both verified). Report + log are backend-tagged; the model default is backend-aware (and configurable per backend). A capability a backend can't honor is rejected before spend, never silently dropped and never framed as one CLI being privileged: codex has no native spend cap (so `--max-budget-usd` is rejected for it), and `--allow-tool` isn't mapped onto codex's own tool model (MCP + sandbox) yet. Open: coarser live progress (items, not char deltas) and the `--allow-tool`→codex bridge (mixed-backend `arc usage` aggregation has landed — codex token-only records are summed and surfaced, never miscounted as no-usage).
- [x] **`arc tui`** — an interactive **cockpit** over the same commands: a launchpad to fire and observe targeted analysis *alongside* the dev agents (not a chat). Inline (preserves the shell's scrollback, not an alt-screen takeover), on a sync input+tick→`mpsc` runtime with render a pure function of state (headless-tested). Landed: the spine (a home masthead with name + version + target directory, a prefix-matched `/` palette, a persistent footer carrying the active-run count across every view, a live `status` view that refreshes in place), a read-only **config** view (resolved settings + active layers, sharing `arc config list`'s projection), and the **launch gate** — selecting a verb spawns its `--dry-run` at zero spend and shows the preview + token/cost estimate in a confirm modal with a hard `Esc`/`Ctrl-C` cancel. Since landed: the **launch** itself — `Enter` at the confirm gate fires the real run in the background (it writes its marker, so `status` and the footer track it live, and logs its result), the cockpit's fire-and-observe model; and a **`log` browse view** — the completed-run history, drill into any run's full detail (params, token usage, context sources, and the verbatim prompt now stored per run). Open: **config editing** (arrow-key pickers, shared with shaping a launch); **shaping a launch's parameters** in the gate before firing (model/include/ruleset/…); the **rules** browse view (its sibling **usage** view since landed); surfacing a *failed* launch inline (a claude run that errors now logs as an *errored* run, so it shows in the `log` view — but the cockpit doesn't yet flag the failure where you launched it, and codex errored runs aren't logged); live-text-streaming a run's output as it goes; and — **the larger aim** — rendering findings and extracted rules as **interactive, structured items acted on in place** (promote a finding into the repo's ledger; pick which rules to merge/dedup), consuming runs' *structured* output rather than rendered text — the natural surface for **Promote findings** and cross-repo **rule aggregation** below.
- [x] **Release pipeline + download distribution** — a `tags: v*` GitHub Pipeline builds per-OS binaries (Linux native; Windows cross-compiled from Linux) and uploads them to the repo's **Downloads** on a version tag — server-side, so coworkers download-and-run with no toolchain. Open: a macOS runner; **`arc update`** + auto-update — check Downloads, notify in the TUI, apply on exit (the agent-CLI model, which sidesteps Windows's can't-overwrite-a-running-exe and arc's concurrent sessions).

**Open:**

- [ ] Aggregate extracted **rules** across repos: dedup rules that recur in 2+ repos, promote them to shared pools, and merge provenance-aware (`extract` produces per-repo candidates today).
- [ ] **Promote findings into the repo** — collect proven results under the repo's `.arc/findings/` (and/or `~/.arc/`) as a curated, per-scope ledger of accepted, still-open findings. **`arc promote <run-id>` landed (2026-06-23):** the *system* writes a logged run's structured findings into `.arc/findings/open/` (atomic, collision-free names — agents invoke arc, they don't hand-author the ledger). **`--findings` landed (2026-06-24):** a run feeds the open ledger into context (framed *surface new issues beyond these*), so it hunts past the already-known. Still open: a **verify lever** that re-checks promoted findings still reproduce (acting on a finding stales it — fixed ones leave the ledger, via the system, not by hand); and cross-scope curation (repo vs. `~/.arc`). Floated 2026-06-10.
- [ ] **Smooth onboarding & rule capture** — initializing and using arclite in any repo should be intuitive end to end: `arc init`, then discovering/extracting the rules a repo already embodies and curating them into the configured ruleset, without friction. First exercised on a .NET UI repo (2026-06-10): init → extract → curate → audit ran end to end and surfaced two gaps, both since closed — `--exclude` (autogenerated files like `.Designer.cs`/`.resx` had eaten much of the included context) and a top-level layout in `arc inspect` for `--include` to aim at. Exercised live again 2026-06-23: a second, active repo's own Claude Code session initialized arc and enabled the pre-push gate itself — the onboarding loop driven by an agent on a live foreign repo, to get real signal on whether the gate earns its keep there (the test of whether this is a feature any dev/repo/agent would actually benefit from). Still open: the broader friction of making the whole loop intuitive without hand-holding.
- [ ] **Surface the available models** — from the CLI (and the TUI): which model ids can be configured, without guessing. Open: how to enumerate them headlessly.
- [ ] **Log errored runs on codex too** — a *claude* run that fails mid-flight (e.g. a tripped `--max-budget-usd` cap) now records its real usage + cost as a logged **errored** run (built from the error payload's `modelUsage` + `total_cost_usd`, since the top-level `usage` block is zeroed on error), so the spend is traced rather than lost — surfaced in `arc log` and counted by `arc usage`. Codex errored runs are still success-only: its result comes from an `-o` artifact, so its error path needs the same usage/cost capture.
- [ ] **Feed VCS-tracking truth into the synthesis context.** The context carries file *contents* but not their git-tracking state, so the model can assert a referenced file is "committed" when it is actually gitignored — surfaced 2026-06-23 when a `self`-ruleset audit of a foreign repo flagged a `.pfx` *referenced* in a committed `.csproj` as a secret "committed into version control", though the key file itself is gitignored and untracked. arclite is already git-aware (the walk is gitignore-aware; `--changed` reads git); surfacing tracked-vs-ignored ground truth into context would let the deterministic layer constrain such VCS-state claims rather than leave the model to infer them — *deterministic until synthesis*.
- [ ] Search across one or more repos.
- [ ] A "lexicon" — canonical project terms + casing that linting enforces (to auto-catch casing/naming drift in product and repo names).
- [ ] Fetch Claude docs → Markdown for citable reference snippets (cite specific lines; *derive* where valuable). Sources under **References**.
- [ ] Fully review arc's codebase + feature set; identify what made sense vs. what was sub-optimal.

## Open questions

- **Rules — format & lifecycle.** The rule format (a **Markdown file**; filename stem = `id`) and ruleset composition ship (see Configuration). Open: frontmatter for *selective inclusion* (`kind`, `scope`, `tags`), added only when something filters on it; rename-stability of filename-ids; whether prompts/todos share the format; and **auditing the ruleset itself for generality** — exercising arc on its own `.arc/rules` so the general-principle discipline (under **Rules as composable levers**) is self-enforced, not only watched by hand. First evidence (2026-06-23): running arclite's own ruleset against a foreign WinForms C# repo produced grounded, language-agnostic findings with no language-bias — empirical support that the rules are general. The literal self-audit of the *rule text* now lands as a composition with no new machinery: a `rule-quality` ruleset — the general-principle discipline expressed as meta-rules under `.arc/meta-rules/` — audited against `.arc/rules`. Its first real run (2026-06-25) scoped correctly to the rule files (not the README also riding in context) and caught two current-state bindings in arclite's own rules — one tying a rule to a sibling rule's existence, one resting a general doc principle on arclite's own design — both since sharpened, with a re-run returning clean. Still open: applying it recursively to the meta-rules themselves; set-level checks (rule redundancy/overlap) beyond the per-rule lens; and whether it earns a place in the per-push loop.
- **Gating / hooks for any command (cost-visible).** The gate (`--fail-on-findings`) and the pre-push hook ship (see **Gating on push**). Still open: **Claude Code hook events** invoking `arc` (complementary to git hooks, not a replacement), and whether a pre-commit variant ever earns its keep. (The tracked hooks now live under `.arc/hooks/`, with the rest of arclite's per-repo footprint.)
- **Command-kit identity.** The commands are one shared substrate differentiated only by prompt (`suggest` surfaces what's worth attention, `critique` finds defects, `audit` checks rules, `summarize` describes, `extract` mines rules, `evolve` proposes radical change). The first five work within the current frame; `evolve` deliberately challenges it. Watch for overlap/necessity/distinctness as the kit grows; let use — not speculation — justify each verb. The verbs now group under `arc run <verb>` (landed): one prompt-differentiated substrate, kept distinct from the deterministic top-level commands, with `run` marking the step that spends AI (per-verb defaults can live in settings).
- **Auto-context depth.** The default context includes the *detected* manifests (root or nested) + the root README. Open: search wider for docs vs. keeping a light default + explicit `--include`. Exercised on IDA (2026-06-23): a no-root-README repo whose overview lives in root `CLAUDE.md` + `ONBOARDING.md` + `docs/` — the default gathered 15 manifests (every `.csproj`/`.sln`/`package.json`, boilerplate-heavy for a 13-project solution) and *no* prose overview at all. Two pulls this surfaced: (a) arc's own **Discovery** goal (integrate agent memory/config) argues the root `CLAUDE.md`/`AGENTS.md` belongs in the default — gathered as an explicit, *reported* source, distinct from the agent's isolated ambient load; (b) pulling *every* nested manifest is mostly boilerplate where the `.sln` already names the projects. **`--no-scan` landed (2026-06-24):** a run can skip the scan baseline (the scan summary + detected manifests, and the walk that builds them), keeping the README + `--include`/`--changed` + rules — so a diff-scoped audit's cost tracks the diff, not a fixed whole-repo baseline. Motivated by a live IDA diff-audit where the 15-manifest baseline was ~the entire cost (a dry run cut the prompt ~57k→~20k chars). Open: whether a lean run should also drop the README, and the wider-docs-search question above.
- **Prompts as files?** Command prompts are inline in code today. Externalizing them as **Markdown** (the same substrate as rules) would make them tunable without a rebuild — do it when a prompt needs tuning without recompiling.
- **Structured output vs. tool calls.** A command's typed result can come two ways both agent CLIs support: a final **structured-output** artifact (today's `--json-schema`/`--output-schema` → `results`+`note`), or the model **calling a tool** with structured *input* (one batch submit, or per-result calls). They're near-equivalent in expressiveness; the difference is lifecycle/protocol (one parse point + whole-report reconciliation vs. per-item events, partial salvage, side effects), not a settled quality claim. arclite uses structured output; tool use is a lever for when a concrete use-case earns it — and an open experiment is whether either channel shifts the *perceived quality* (or other aspects) of a run.
- **Reclaim the `arc` name.** The binary is *already* `arc` (it shadows any legacy `arc` at install time); the open part is renaming the repo (here + on GitHub) and formally superseding legacy arc.
- **Agent-agnostic?** (Claude Code + Codex + any.) **Distribution** — per-OS binaries on Downloads (via the release pipeline) ship now; still open: `cargo-binstall`/Homebrew/Scoop, and `arc update`/auto-update. **CI** — Linux build/test/lint + the tagged release run are in place; macOS needs a runner (Windows is cross-compiled from Linux). **Dashboard / IDE / linter integration?**

## Related repositories

- <https://github.com/nikganderson/arc/src/> — legacy arc (deprecated — its README banners the supersession and the MCP/hook cleanup steps; reference-only).
- <https://github.com/nikganderson/ida/src/> — IDA, a live repo arclite is exercised against.
- <https://github.com/nikganderson/quant/src/> — quant, likewise.

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
