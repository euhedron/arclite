//! Shared synthesis runner for AI-backed commands (`summarize`, `suggest`, …).
//!
//! `--dry-run` previews the prompt + estimate at zero spend; real calls report actual cost + cache
//! usage; and every run echoes the full parameter set it used (model, tools, context sources).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

use crate::ai;
use crate::output::emit;

/// Default model when `--model` is omitted. Update when a newer model supersedes it; the run
/// reports the resolved id the response returns.
const DEFAULT_MODEL: &str = "claude-opus-4-8";

const DRY_RUN_NOTE: &str = "estimate counts the prompt only; a real call also loads the model's base system/tool context (not counted here) — actual usage is reported after the call runs";

/// Exit code when an opt-in gate (`--fail-on-findings`) blocks — distinct from `1` (arclite error)
/// so a hook/CI can tell "found violations" apart from "the tool failed". Any non-zero blocks.
const GATE_BLOCKED_EXIT: u8 = 2;

const LOGGING_OFF_NOTE: &str = "\nlogging: off (defaults.logging = false)";

/// The single key for every command's structured output: a `results` array. Defined once — the
/// schema is built from it ([`results_schema`]), the gate reads it, and multi-run unions it — so it
/// can't drift across schema and code.
pub(crate) const RESULTS_KEY: &str = "results";

/// Wrap a command's array-item schema in the shared `{ results: [ <item> ] }` envelope, so each
/// command declares only its item shape. The CLI's structured output requires a root object (a
/// top-level array is rejected — confirmed by exercise), so the list can't be the root.
pub(crate) fn results_schema(item: &str) -> String {
    format!(
        r#"{{"type":"object","properties":{{"{RESULTS_KEY}":{{"type":"array","items":{item}}}}},"required":["{RESULTS_KEY}"]}}"#
    )
}

/// Configuration shared by every synthesis-backed command.
pub struct SynthOptions<'a> {
    /// Model id; `None` uses [`DEFAULT_MODEL`].
    pub model: Option<&'a str>,
    /// Number of synthesis runs to fan out concurrently; their results are unioned. 1 = single run.
    pub runs: usize,
    /// Claude tools to allow (empty = none).
    pub allowed_tools: &'a [String],
    /// Repository root, granted to allowed tools via `--add-dir` (the working directory is otherwise neutral).
    pub dir: &'a Path,
    /// Human-readable descriptions of the context pieces included (for the run report).
    pub sources: &'a [String],
    /// Notable context excluded by default (e.g. source files), surfaced so defaults aren't hidden.
    pub excluded: &'a [String],
    /// The active `.arc/settings.json` layers (user then project); empty = built-in defaults only.
    pub config: &'a [String],
    /// Command name (e.g. "suggest") — names the `--output` file and labels the doc's provenance.
    pub command: &'a str,
    /// Optional directory to also write the synthesis into, as `<command>.md`.
    pub output: Option<&'a Path>,
    /// Load the Claude CLI's ambient user/project memory instead of isolating (default: isolate).
    pub ambient_memory: bool,
    /// JSON Schema for structured output (`--structured`), or `None` for free-form prose.
    pub schema: Option<&'a str>,
    /// When `Some(field)` (from `--fail-on-findings`), block — exit non-zero — if the structured
    /// output's `field` array is non-empty. `None` = no gating (default). Decoupled policy: the
    /// synthesis is unchanged; only the process exit code (and a loud line) reflect the gate.
    pub gate: Option<&'a str>,
    /// Preview the prompt + estimate without calling the model (zero spend).
    pub dry_run: bool,
    /// Emit machine-readable JSON instead of human text.
    pub json: bool,
    /// Append a record of this run to `~/.arc/logs/runs.jsonl` (real runs only; disable via settings).
    pub log: bool,
}

/// A file's text, optionally capped: the body, its original char count, and the cap it
/// was truncated to (if any) — so any truncation is reportable, never silent.
struct Capped {
    body: String,
    original_chars: usize,
    truncated_to: Option<usize>,
}

/// Read a file as text. `max` is an *optional, caller-chosen* cap (a compression knob);
/// by default (`None`) the whole file is read — context elision is never automatic.
fn read_file(path: &Path, max: Option<usize>) -> Option<Capped> {
    let text = std::fs::read_to_string(path).ok()?;
    let original_chars = text.chars().count();
    match max {
        Some(cap) if original_chars > cap => Some(Capped {
            body: format!(
                "{}\n…[truncated by arclite at {cap} chars]",
                text.chars().take(cap).collect::<String>()
            ),
            original_chars,
            truncated_to: Some(cap),
        }),
        _ => Some(Capped {
            body: text,
            original_chars,
            truncated_to: None,
        }),
    }
}

/// A `sources` label for a file, making any caller-applied truncation explicit.
fn source_label(name: impl std::fmt::Display, cap: &Capped) -> String {
    match cap.truncated_to {
        Some(to) => format!("{name} ({} chars, truncated to {to})", cap.original_chars),
        None => format!("{name} ({} chars)", cap.original_chars),
    }
}

/// A `sources` label for a present-but-unreadable file — single-sourced so the wording can't drift.
fn unreadable_label(label: &str) -> String {
    format!("{label} (unreadable — skipped)")
}

/// Walk a directory gitignore-aware, returning its files (sorted; `.git` skipped) and the
/// count of walk errors (unreadable entries) so callers can surface them, not drop them.
fn walk_files(dir: &Path) -> (Vec<PathBuf>, usize) {
    let (entries, errors) = crate::walk::entries(dir);
    let mut files: Vec<PathBuf> = entries
        .into_iter()
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .collect();
    files.sort();
    (files, errors)
}

/// Expand each `--include` path (a file *or* a directory) into context text, applying the
/// optional caller cap and skipping anything already in `seen` (canonical paths of files
/// auto-added as README/manifests) so they aren't double-counted. Dirs walked gitignore-aware.
fn gather_includes(
    paths: &[PathBuf],
    max: Option<usize>,
    seen: &[PathBuf],
    sources: &mut Vec<String>,
) -> String {
    let already_seen = |p: &Path| {
        std::fs::canonicalize(p)
            .map(|c| seen.contains(&c))
            .unwrap_or(false)
    };
    let mut ctx = String::new();
    for path in paths {
        let is_dir = path.is_dir();
        let (files, walk_errors) = if is_dir {
            walk_files(path)
        } else {
            (vec![path.clone()], 0)
        };
        let mut unreadable = 0usize;
        for file in &files {
            if already_seen(file) {
                continue; // already auto-included (README/manifest) — don't double-count
            }
            let label = file.display().to_string();
            if !append_file(&label, file, max, &mut ctx, sources) {
                if is_dir {
                    unreadable += 1;
                } else {
                    sources.push(unreadable_label(&label));
                }
            }
        }
        // Surface unreadable files walked under a directory, rather than silently dropping them.
        if unreadable > 0 {
            sources.push(format!(
                "{unreadable} unreadable file(s) under {} — skipped",
                path.display()
            ));
        }
        // Likewise surface entries the walk itself couldn't read (permission denied, I/O, …).
        if walk_errors > 0 {
            sources.push(format!(
                "{walk_errors} unwalkable entr(ies) under {} — skipped",
                path.display()
            ));
        }
    }
    ctx
}

/// Render the rules from `rule_sources` as a context block, recording which rule ids were included.
fn gather_rules(rule_sources: &[PathBuf], sources: &mut Vec<String>) -> anyhow::Result<String> {
    if rule_sources.is_empty() {
        return Ok(String::new());
    }
    let (rules, skipped) = crate::rules::load_sources(rule_sources)?;
    for src in &skipped {
        // A configured source that resolved to nothing (typo'd path, absent dir, or a non-`.md`
        // file): surface it in the manifest so a shrunken ruleset never goes unnoticed.
        sources.push(format!(
            "rules: source skipped — not a directory or .md file: {}",
            src.display()
        ));
    }
    if rules.is_empty() {
        sources.push(format!(
            "rules: none found in {} source(s)",
            rule_sources.len()
        ));
        return Ok(String::new());
    }
    let ids = rules
        .iter()
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    sources.push(format!("rules ({}): {ids}", rules.len()));
    Ok(format!("\nRules:\n{}\n", crate::rules::render(&rules)))
}

/// Read `path` (capped at `max`) and, on success, append its body to `text` under `label` and record
/// the source label. Returns whether it was read; the caller surfaces a miss however it needs.
fn append_file(
    label: &str,
    path: &Path,
    max: Option<usize>,
    text: &mut String,
    sources: &mut Vec<String>,
) -> bool {
    let Some(cap) = read_file(path, max) else {
        return false;
    };
    sources.push(source_label(label, &cap));
    text.push_str(&format!("\n{label}:\n{}\n", cap.body));
    true
}

/// Append `path` to the context as `label` (recording its canonical path so `--include` won't
/// double-count it); a present-but-unreadable file is surfaced, an absent one stays silent.
fn add_file(
    path: &Path,
    label: &str,
    max: Option<usize>,
    text: &mut String,
    sources: &mut Vec<String>,
    seen: &mut Vec<PathBuf>,
) {
    if append_file(label, path, max, text, sources) {
        if let Ok(c) = std::fs::canonicalize(path) {
            seen.push(c);
        }
    } else if path.exists() {
        sources.push(unreadable_label(label));
    }
}

/// Assembled repo context, a record of every source and what was excluded, and the repo
/// root (granted to tools when any are allowed).
pub struct Context {
    pub text: String,
    pub sources: Vec<String>,
    pub excluded: Vec<String>,
    pub root: PathBuf,
}

/// Files with uncommitted changes (staged, unstaged, or untracked) under `root`, per git —
/// backing `--changed`. `Ok(vec)` lists them (empty = genuinely clean tree); `Err(reason)`
/// means git itself couldn't be consulted — kept distinct so a failed scope never masquerades
/// as a clean "no changes" result.
fn changed_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = ai::command("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git exited with {} (is {} a git repository?)",
            output.status,
            root.display()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // porcelain: two status chars, a space, then the path — "old -> new" for renames.
            let path = line.get(3..)?.trim().trim_matches('"');
            let path = path.rsplit_once(" -> ").map_or(path, |(_, new)| new);
            (!path.is_empty()).then(|| root.join(path))
        })
        .collect())
}

/// Assemble the standard repo context shared by every synthesis command: the scan summary,
/// README + manifest bodies, any `--include`d files/dirs (and, with `changed`, git-changed
/// files), and rules — tracking each source (and what's excluded) for the run report. `max`
/// is the optional caller cap; by default files are read whole. Commands differ only in the prompt.
pub fn gather_context(
    path: &Path,
    includes: &[PathBuf],
    rule_sources: &[PathBuf],
    max: Option<usize>,
    changed: bool,
) -> anyhow::Result<Context> {
    let (report, root) = crate::commands::inspect::gather(path)?;

    let mut text = format!(
        "Repository scan (JSON):\n{}\n",
        serde_json::to_string_pretty(&report)?
    );
    let mut sources = vec!["repository scan".to_owned()];
    let mut seen: Vec<PathBuf> = Vec::new();

    add_file(
        &root.join("README.md"),
        "README.md",
        max,
        &mut text,
        &mut sources,
        &mut seen,
    );
    // Include the manifests inspect actually detected (root *or* nested) — keeping the scan's
    // findings and the synthesis context consistent, and making default runs substantive on real
    // repos whose manifests live in subprojects, not at the root.
    for rel in &report.manifest_paths {
        add_file(
            &root.join(rel),
            rel,
            max,
            &mut text,
            &mut sources,
            &mut seen,
        );
    }
    // Resolve --include paths against the target repo (not arclite's cwd); absolute paths as-is.
    let mut includes: Vec<PathBuf> = includes
        .iter()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                root.join(p)
            }
        })
        .collect();
    // --changed: scope to git-changed files — same group as --include, not special to any command.
    // A git failure aborts loudly rather than silently passing as a clean tree.
    if changed {
        let files = changed_files(&root)
            .map_err(|reason| anyhow::anyhow!("--changed could not consult git: {reason}"))?;
        sources.push(if files.is_empty() {
            "changed: no git changes found".to_owned()
        } else {
            format!("changed: {} git-changed file(s)", files.len())
        });
        includes.extend(files);
    }
    text.push_str(&gather_includes(&includes, max, &seen, &mut sources));
    text.push_str(&gather_rules(rule_sources, &mut sources)?);

    let excluded = if includes.is_empty() {
        vec!["the repo's source files (--include <path> or --changed to add)".to_owned()]
    } else {
        Vec::new()
    };

    Ok(Context {
        text,
        sources,
        excluded,
        root,
    })
}

/// The exact parameters a run used — reported alongside every result so nothing is opaque.
#[derive(Serialize)]
struct RunReport<'a> {
    model: String,
    /// How many synthesis runs were combined (1 = single; >1 = concurrent multi-run, unioned).
    runs: usize,
    tools: Vec<&'a str>,
    /// "isolated" (default — no ambient CLAUDE.md/auto-memory) or "ambient" (loaded). Surfaced
    /// because it shapes what the model sees: "isolated" means the context list below is authoritative.
    memory: &'a str,
    context: &'a [String],
    excluded: &'a [String],
    /// Active `.arc/settings.json` layers in effect for this run (empty = built-in defaults only).
    config: &'a [String],
    /// The findings field this run gates on (`--fail-on-findings`), or `None`. Recorded as a run
    /// parameter (for `--json`); the pass/block outcome is shown on its own line to stay legible.
    gate: Option<&'a str>,
}

impl RunReport<'_> {
    fn human(&self) -> String {
        let tools = if self.tools.is_empty() {
            "none".to_owned()
        } else {
            self.tools.join(",")
        };
        let runs = if self.runs > 1 {
            format!("  runs={}", self.runs)
        } else {
            String::new()
        };
        let mut line = format!(
            "model={}{}  tools={}  memory={}  context=[{}]",
            self.model,
            runs,
            tools,
            self.memory,
            self.context.join(", ")
        );
        if !self.excluded.is_empty() {
            line.push_str(&format!("  excluded=[{}]", self.excluded.join(", ")));
        }
        line.push_str(&format!(
            "\nconfig: {}",
            if self.config.is_empty() {
                "built-in defaults (no .arc/settings.json active)".to_owned()
            } else {
                self.config.join(", ")
            }
        ));
        line
    }
}

#[derive(Serialize)]
struct SynthOutput<'a> {
    run: RunReport<'a>,
    synthesis: String,
    usage: ai::Usage,
    /// Path written by `--output`, if any (so the run report says where the doc went).
    output: Option<String>,
    /// Path of the run log this run was appended to, if logging was on (disclosed every run).
    log: Option<String>,
    /// Schema-validated structured result, if `--structured` was used.
    structured: Option<serde_json::Value>,
}

/// One line of `~/.arc/logs/runs.jsonl`: the durable, machine-readable trace of a real run — its
/// params, context sources, and ground-truth token usage + cost. Dry runs are never logged (no
/// spend, no call). The full [`ai::Usage`] is nested verbatim, so token/cost fields single-source.
#[derive(Serialize)]
struct RunRecord<'a> {
    ts: u64,
    command: &'a str,
    repo: String,
    model: &'a str,
    runs: usize,
    memory: &'a str,
    structured: bool,
    sources: &'a [String],
    usage: &'a ai::Usage,
    /// The findings field gated on (`--fail-on-findings`), or `None` — with `blocked`, this lets
    /// metrics ask "how often does the gate actually block?" (the spend-vs-value question).
    gate: Option<&'a str>,
    blocked: bool,
}

#[derive(Serialize)]
struct DryRunOutput<'a> {
    dry_run: bool,
    run: RunReport<'a>,
    estimate: ai::Estimate,
    note: &'static str,
    /// Where `--output` *would* write on a real run, if set.
    output_target: Option<String>,
    prompt: &'a str,
}

/// Preview (dry-run) or run a synthesis prompt, echoing the full run parameters.
pub fn run(prompt: &str, opts: &SynthOptions) -> anyhow::Result<ExitCode> {
    let requested = opts.model.unwrap_or(DEFAULT_MODEL);
    // The report names the model that actually ran (set from the response after the call); until
    // then it holds the requested model — all a dry run can name, since nothing runs.
    let mut report = RunReport {
        model: requested.to_owned(),
        runs: opts.runs,
        tools: opts.allowed_tools.iter().map(String::as_str).collect(),
        memory: if opts.ambient_memory {
            "ambient"
        } else {
            "isolated"
        },
        context: opts.sources,
        excluded: opts.excluded,
        config: opts.config,
        gate: opts.gate,
    };

    if opts.dry_run {
        let estimate = ai::estimate(prompt);
        let output_target = opts.output.map(|dir| {
            dir.join(format!("{}.md", opts.command))
                .display()
                .to_string()
        });
        let mut human = format!(
            "[dry run]\nrun: {}\nprompt: {} chars (~{} tokens)\nnote: {}",
            report.human(),
            estimate.chars,
            estimate.approx_tokens,
            DRY_RUN_NOTE,
        );
        if let Some(target) = &output_target {
            human.push_str(&format!("\noutput: would write {target} on a real run"));
        }
        if opts.schema.is_some() {
            human.push_str("\nstructured: on — the result will be a schema-validated object");
        }
        if let Some(field) = opts.gate {
            human.push_str(&format!(
                "\ngate: on — a real run exits {GATE_BLOCKED_EXIT} if `{field}` is non-empty"
            ));
        }
        // Disclose logging status + where real runs would record, even though a dry run logs nothing.
        match (opts.log, crate::log::path()) {
            (true, Some(path)) => {
                human.push_str(&format!("\nlogging: on — real runs append to {}", path.display()));
            }
            (false, _) => human.push_str(LOGGING_OFF_NOTE),
            (true, None) => {}
        }
        human.push_str(&format!("\n\n{prompt}"));
        let out = DryRunOutput {
            dry_run: true,
            run: report,
            estimate,
            note: DRY_RUN_NOTE,
            output_target,
            prompt,
        };
        emit(&out, &human, opts.json)?;
        return Ok(ExitCode::SUCCESS);
    }

    // Record this run in the active-run registry; the guard clears it on exit (success or error).
    let _active = crate::runs::register(opts.command, opts.dir, requested);
    let (synthesis, runs) = if opts.runs > 1 {
        multi_synthesize(prompt, requested, opts)?
    } else {
        let single = ai::synthesize(
            prompt,
            requested,
            opts.allowed_tools,
            opts.dir,
            opts.ambient_memory,
            opts.schema,
        )?;
        (single, 1)
    };
    let usage = synthesis.usage;
    // From here the report reflects the model the response says ran, and how many runs were combined.
    report.model = usage.model.clone();
    report.runs = runs;
    let structured = synthesis.structured;
    let text = synthesis.text;
    let cost = format!("${:.4}", usage.cost_usd);
    // Display body: the validated structured object (pretty-printed) when present, else the prose.
    let body = match &structured {
        Some(value) => serde_json::to_string_pretty(value).expect("a serde_json::Value re-serializes"),
        None => text.clone(),
    };
    // Count the gated findings before `structured` is moved out. The schema guarantees the field is
    // a present array; a missing one is the CLI ignoring the requested schema — an error, not a 0-pass.
    let gate_findings = match opts.gate {
        Some(field) => Some(
            structured
                .as_ref()
                .and_then(|v| v.get(field))
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| anyhow::anyhow!("gated on `{field}` but the result has no `{field}` array"))?
                .len(),
        ),
        None => None,
    };
    let gate_blocked = gate_findings.is_some_and(|n| n > 0);
    // --output: also write the result as a doc with a provenance header.
    let written = match opts.output {
        Some(dir) => Some(write_output(
            dir,
            opts.command,
            &body,
            &report.model,
            opts.sources.len(),
            &cost,
        )?),
        None => None,
    };
    let mut human = format!(
        "{}\n\nrun: {}\ncost: in {}  cache-write {}  cache-read {}  out {} | {}",
        body,
        report.human(),
        usage.input_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens,
        usage.output_tokens,
        cost,
    );
    if let Some(path) = &written {
        human.push_str(&format!("\nwrote: {}", path.display()));
    }
    // Gate outcome on its own line so a blocked commit is unmistakable (real run only — the dry-run
    // path notes that gating is armed instead). Shown for pass too, so "on and clean" isn't silent.
    if let (Some(field), Some(n)) = (opts.gate, gate_findings) {
        human.push_str(&format!(
            "\ngate: {} — {n} `{field}`{}",
            if gate_blocked { "BLOCKED" } else { "passed" },
            if gate_blocked {
                format!(" (exit {GATE_BLOCKED_EXIT})")
            } else {
                String::new()
            },
        ));
    }
    // Append a durable run record (real runs only) before emitting — observability that outlives
    // the terminal scrollback. A logging failure warns but never fails the command.
    let logged = if opts.log {
        crate::log::append(&RunRecord {
            ts: crate::log::now_secs(),
            command: opts.command,
            repo: opts.dir.display().to_string(),
            model: &report.model,
            runs,
            memory: report.memory,
            structured: opts.schema.is_some(),
            sources: opts.sources,
            usage: &usage,
            gate: opts.gate,
            blocked: gate_blocked,
        })
    } else {
        None
    };
    // Disclose where the run was logged (or that logging is off) — the log location is never hidden.
    match &logged {
        Some(path) => human.push_str(&format!("\nlogged: {}", path.display())),
        None if !opts.log => human.push_str(LOGGING_OFF_NOTE),
        None => {} // logging on but the append failed — append() already warned to stderr
    }
    let out = SynthOutput {
        run: report,
        synthesis: text,
        usage,
        output: written.map(|p| p.display().to_string()),
        log: logged.map(|p| p.display().to_string()),
        structured,
    };
    emit(&out, &human, opts.json)?;
    // The gate's verdict is the process exit code (distinct from error) so a hook enforces on status
    // alone; SUCCESS when not gating or when the findings collection is empty.
    Ok(if gate_blocked {
        ExitCode::from(GATE_BLOCKED_EXIT)
    } else {
        ExitCode::SUCCESS
    })
}

/// Run the synthesis `opts.runs` times concurrently and combine the outcomes, returning the combined
/// result and how many runs succeeded. A failed run is surfaced and skipped; only an all-fail errors.
fn multi_synthesize(
    prompt: &str,
    model: &str,
    opts: &SynthOptions,
) -> anyhow::Result<(ai::Synthesis, usize)> {
    let n = opts.runs;
    let outcomes: Vec<anyhow::Result<ai::Synthesis>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..n)
            .map(|_| {
                scope.spawn(|| {
                    ai::synthesize(
                        prompt,
                        model,
                        opts.allowed_tools,
                        opts.dir,
                        opts.ambient_memory,
                        opts.schema,
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a synthesis thread panicked"))
            .collect()
    });

    let mut ok = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(s) => ok.push(s),
            Err(e) => eprintln!("arclite: a run failed and was skipped: {e:#}"),
        }
    }
    anyhow::ensure!(!ok.is_empty(), "all {n} runs failed");
    let succeeded = ok.len();
    Ok((combine_runs(ok), succeeded))
}

/// Combine successful runs: sum their usage, then union the structured `results` (deduped) — or, for
/// prose commands, present each run's text in turn.
fn combine_runs(runs: Vec<ai::Synthesis>) -> ai::Synthesis {
    let usage = sum_usage(&runs);
    if runs.iter().all(|r| r.structured.is_some()) {
        let combined = union_results(runs.iter().filter_map(|r| r.structured.as_ref()));
        let text =
            serde_json::to_string_pretty(&combined).expect("a serde_json::Value re-serializes");
        ai::Synthesis {
            text,
            usage,
            structured: Some(combined),
        }
    } else {
        let text = runs
            .iter()
            .enumerate()
            .map(|(i, r)| format!("— run {} —\n{}", i + 1, r.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        ai::Synthesis {
            text,
            usage,
            structured: None,
        }
    }
}

/// Union the `results` arrays of several structured outputs into one, deduping identical items.
/// Generic over the item shape, so it serves repeats of one command and (later) different commands.
fn union_results<'a>(
    structured: impl Iterator<Item = &'a serde_json::Value>,
) -> serde_json::Value {
    let mut pooled: Vec<serde_json::Value> = Vec::new();
    for value in structured {
        if let Some(items) = value.get(RESULTS_KEY).and_then(serde_json::Value::as_array) {
            for item in items {
                if !pooled.contains(item) {
                    pooled.push(item.clone());
                }
            }
        }
    }
    let mut obj = serde_json::Map::new();
    obj.insert(RESULTS_KEY.to_owned(), serde_json::Value::Array(pooled));
    serde_json::Value::Object(obj)
}

/// Sum token usage and cost across runs; the model is the same across them, so take the first's.
fn sum_usage(runs: &[ai::Synthesis]) -> ai::Usage {
    let mut total = runs[0].usage.clone();
    for run in &runs[1..] {
        total.input_tokens += run.usage.input_tokens;
        total.output_tokens += run.usage.output_tokens;
        total.cache_creation_input_tokens += run.usage.cache_creation_input_tokens;
        total.cache_read_input_tokens += run.usage.cache_read_input_tokens;
        total.cost_usd += run.usage.cost_usd;
    }
    total
}

/// Write the synthesis to `<dir>/<command>.md` with a provenance header (model, sources, cost).
/// Returns the path.
fn write_output(
    dir: &Path,
    command: &str,
    text: &str,
    model: &str,
    n_sources: usize,
    cost: &str,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("could not create output dir {}: {e}", dir.display()))?;
    let path = dir.join(format!("{command}.md"));
    let body = format!(
        "<!-- Generated by `arc {command}` — model={model}, {n_sources} context source(s), cost {cost}. Self-derived; do not hand-maintain. -->\n\n{text}\n"
    );
    std::fs::write(&path, &body)
        .map_err(|e| anyhow::anyhow!("could not write {}: {e}", path.display()))?;
    Ok(path)
}
