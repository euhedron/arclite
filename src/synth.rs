//! Shared synthesis runner for AI-backed commands (`summarize`, `suggest`, …).
//!
//! Keeps every such command cost-transparent and self-describing: `--dry-run`
//! previews the exact prompt + estimate at zero spend, real calls report actual
//! cost + cache usage, and **every run echoes the full parameter set it used**
//! (model, tools, context sources) so output is never judged blind to its setup.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::ai;
use crate::output::emit;

/// Default model — the best available. Configure *down* via `--model` only when
/// deliberately trading quality for cost; a small model gives unrealistic signal.
const DEFAULT_MODEL: &str = "opus";

const DRY_RUN_NOTE: &str = "estimate counts the prompt only; a real call also loads the model's base system/tool context, which typically dominates the cost — actual usage is reported after the call runs";

/// Configuration shared by every synthesis-backed command.
pub struct SynthOptions<'a> {
    /// Model id; `None` uses [`DEFAULT_MODEL`].
    pub model: Option<&'a str>,
    /// Claude tools to allow (empty = none = cheapest; a cost lever). When non-empty the
    /// run is granted read access to `dir`, so the tools can actually reach the repo.
    pub allowed_tools: &'a [String],
    /// Repository root, granted to allowed tools via `--add-dir`.
    pub dir: &'a Path,
    /// Human-readable descriptions of the context pieces included (for the run report).
    pub sources: &'a [String],
    /// Notable context excluded by default (e.g. source files), surfaced so defaults aren't hidden.
    pub excluded: &'a [String],
    /// Preview the prompt + estimate without calling the model (zero spend).
    pub dry_run: bool,
    /// Emit machine-readable JSON instead of human text.
    pub json: bool,
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

/// Walk a directory gitignore-aware, returning its files (sorted; `.git` skipped) and the
/// count of walk errors (unreadable entries) so callers can surface them, not drop them.
fn walk_files(dir: &Path) -> (Vec<PathBuf>, usize) {
    let (entries, errors) = crate::walk::entries(dir);
    let mut files: Vec<PathBuf> = entries
        .into_iter()
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .filter(|p| !crate::walk::in_git_dir(p))
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
            match read_file(file, max) {
                Some(cap) => {
                    sources.push(source_label(file.display(), &cap));
                    ctx.push_str(&format!("\n{}:\n{}\n", file.display(), cap.body));
                }
                None if !is_dir => {
                    sources.push(format!("{} (unreadable — skipped)", file.display()));
                }
                None => unreadable += 1,
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

/// Render the Markdown rules in `dir` (if any) as a context block, noting the source.
fn gather_rules(dir: Option<&Path>, sources: &mut Vec<String>) -> anyhow::Result<String> {
    let Some(dir) = dir else {
        return Ok(String::new());
    };
    let Some(text) = crate::rules::block(Some(dir))? else {
        return Ok(String::new());
    };
    sources.push(format!("rules: {}", dir.display()));
    Ok(format!("\nRules:\n{text}\n"))
}

/// Read `path` (capped) and, if present, append it to the context as `label` — recording the
/// source and its canonical path so `--include` won't double-count it.
fn add_file(
    path: &Path,
    label: &str,
    max: Option<usize>,
    text: &mut String,
    sources: &mut Vec<String>,
    seen: &mut Vec<PathBuf>,
) {
    if let Some(cap) = read_file(path, max) {
        sources.push(source_label(label, &cap));
        text.push_str(&format!("\n{label}:\n{}\n", cap.body));
        if let Ok(c) = std::fs::canonicalize(path) {
            seen.push(c);
        }
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
/// as a clean "no changes" result (the no-silent-defaults rule).
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
            // porcelain: two status chars, a space, then the path ("old -> new" for renames).
            let path = line.get(3..)?.trim().trim_matches('"');
            let path = path.rsplit(" -> ").next().unwrap_or(path);
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
    rules_dir: Option<&Path>,
    max: Option<usize>,
    changed: bool,
) -> anyhow::Result<Context> {
    let report = crate::commands::inspect::gather(path)?;
    let root = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());

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
    for &name in crate::commands::inspect::MANIFEST_NAMES {
        add_file(
            &root.join(name),
            name,
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
    // A git failure aborts loudly rather than silently passing as a clean tree (no-silent-defaults).
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
    text.push_str(&gather_rules(rules_dir, &mut sources)?);

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
    model: &'a str,
    tools: Vec<&'a str>,
    context: &'a [String],
    excluded: &'a [String],
}

impl RunReport<'_> {
    fn human(&self) -> String {
        let tools = if self.tools.is_empty() {
            "none".to_owned()
        } else {
            self.tools.join(",")
        };
        let mut line = format!(
            "model={}  tools={}  context=[{}]",
            self.model,
            tools,
            self.context.join(", ")
        );
        if !self.excluded.is_empty() {
            line.push_str(&format!("  excluded=[{}]", self.excluded.join(", ")));
        }
        line
    }
}

#[derive(Serialize)]
struct SynthOutput<'a> {
    run: RunReport<'a>,
    synthesis: String,
    usage: ai::Usage,
}

#[derive(Serialize)]
struct DryRunOutput<'a> {
    dry_run: bool,
    run: RunReport<'a>,
    estimate: ai::Estimate,
    note: &'static str,
    prompt: &'a str,
}

/// Preview (dry-run) or run a synthesis prompt, echoing the full run parameters.
pub fn run(prompt: &str, opts: &SynthOptions) -> anyhow::Result<()> {
    let model = opts.model.unwrap_or(DEFAULT_MODEL);
    let report = RunReport {
        model,
        tools: opts.allowed_tools.iter().map(String::as_str).collect(),
        context: opts.sources,
        excluded: opts.excluded,
    };

    if opts.dry_run {
        let estimate = ai::estimate(prompt);
        let human = format!(
            "[dry run — no AI call, $0.00]\nrun: {}\nprompt: {} chars (~{} tokens)\nnote: {}\n\n{}",
            report.human(),
            estimate.chars,
            estimate.approx_tokens,
            DRY_RUN_NOTE,
            prompt,
        );
        let out = DryRunOutput {
            dry_run: true,
            run: report,
            estimate,
            note: DRY_RUN_NOTE,
            prompt,
        };
        return emit(&out, &human, opts.json);
    }

    let synthesis = ai::synthesize(prompt, model, opts.allowed_tools, opts.dir)?;
    let usage = synthesis.usage;
    // Cost comes straight from the CLI (ground truth); show "unknown" if it ever omits it.
    let cost = usage
        .cost_usd
        .map_or_else(|| "unknown".to_owned(), |c| format!("${c:.4}"));
    let human = format!(
        "{}\n\nrun: {}\ncost: in {}  cache-write {}  cache-read {}  out {} | {}",
        synthesis.text,
        report.human(),
        usage.input_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens,
        usage.output_tokens,
        cost,
    );
    let out = SynthOutput {
        run: report,
        synthesis: synthesis.text,
        usage,
    };
    emit(&out, &human, opts.json)
}
