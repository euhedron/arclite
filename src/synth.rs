//! Shared synthesis runner for AI-backed commands (`summarize`, `suggest`, …).
//!
//! Keeps every such command cost-transparent and self-describing: `--dry-run`
//! previews the exact prompt + estimate at zero spend, real calls report actual
//! cost + cache usage, and **every run echoes the full parameter set it used**
//! (model, tools, context sources) so output is never judged blind to its setup.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use serde::Serialize;

use crate::ai;
use crate::output::emit;

/// Default model — the best available. Configure *down* via `--model` only when
/// deliberately trading quality for cost; a small model gives unrealistic signal.
const DEFAULT_MODEL: &str = "opus";

/// Per-file char cap and total char budget across all `--include` paths, keeping
/// context (and cost) bounded; truncation is always surfaced in the run's sources.
const INCLUDE_FILE_CAP: usize = 4_000;
const INCLUDE_TOTAL_BUDGET: usize = 60_000;

const DRY_RUN_NOTE: &str = "estimate counts the prompt only; a real call also loads the model's base system/tool context, which typically dominates the cost — actual usage is reported after the call runs";

/// Configuration shared by every synthesis-backed command.
pub struct SynthOptions<'a> {
    /// Model id; `None` uses [`DEFAULT_MODEL`].
    pub model: Option<&'a str>,
    /// Claude tools to allow (empty = none = cheapest; a cost lever).
    pub allowed_tools: &'a [String],
    /// Human-readable descriptions of the context pieces included (for the run report).
    pub sources: &'a [String],
    /// Notable context excluded by default (e.g. source files), surfaced so defaults aren't hidden.
    pub excluded: &'a [String],
    /// Preview the prompt + estimate without calling the model (zero spend).
    pub dry_run: bool,
    /// Emit machine-readable JSON instead of human text.
    pub json: bool,
}

/// Read a file as text, capped at `max_chars` (char-safe) to keep context bounded.
pub fn read_capped(path: &Path, max_chars: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.chars().count() > max_chars {
        Some(format!(
            "{}\n…[truncated]",
            text.chars().take(max_chars).collect::<String>()
        ))
    } else {
        Some(text)
    }
}

/// Walk a directory gitignore-aware, returning its files (sorted; `.git` skipped).
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkBuilder::new(dir)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .build()
        .flatten()
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .filter(|p| !p.components().any(|c| c.as_os_str() == ".git"))
        .collect();
    files.sort();
    files
}

/// Expand each `--include` path (a file *or* a directory) into capped context text,
/// pushing a human description of every file included (or skipped) onto `sources`.
/// Directories are walked gitignore-aware; the total is bounded and any truncation reported.
pub fn gather_includes(paths: &[PathBuf], sources: &mut Vec<String>) -> String {
    let mut ctx = String::new();
    let mut used = 0usize;
    for path in paths {
        let is_dir = path.is_dir();
        let files = if is_dir {
            walk_files(path)
        } else {
            vec![path.clone()]
        };
        let total = files.len();
        let mut included = 0usize;
        for file in &files {
            if used >= INCLUDE_TOTAL_BUDGET {
                break;
            }
            match read_capped(file, INCLUDE_FILE_CAP) {
                Some(body) => {
                    used += body.chars().count();
                    included += 1;
                    sources.push(format!(
                        "{} ({} chars)",
                        file.display(),
                        body.chars().count()
                    ));
                    ctx.push_str(&format!("\n{}:\n{body}\n", file.display()));
                }
                None if !is_dir => {
                    sources.push(format!("{} (unreadable — skipped)", file.display()));
                }
                None => {}
            }
        }
        if is_dir && included < total {
            sources.push(format!(
                "…{included} of {total} files under {} included (rest skipped: budget or unreadable)",
                path.display()
            ));
        }
    }
    ctx
}

/// Render the Markdown rules in `dir` (if any) as a context block, noting the source.
pub fn gather_rules(dir: Option<&Path>, sources: &mut Vec<String>) -> anyhow::Result<String> {
    let Some(dir) = dir else {
        return Ok(String::new());
    };
    let Some(text) = crate::rules::block(Some(dir))? else {
        return Ok(String::new());
    };
    sources.push(format!("rules: {}", dir.display()));
    Ok(format!("\nRules:\n{text}\n"))
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

    let synthesis = ai::synthesize(prompt, model, opts.allowed_tools)?;
    let usage = synthesis.usage;
    let human = format!(
        "{}\n\nrun: {}\ncost: in {}  cache-write {}  cache-read {}  out {} | ${:.4}",
        synthesis.text,
        report.human(),
        usage.input_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens,
        usage.output_tokens,
        usage.cost_usd,
    );
    let out = SynthOutput {
        run: report,
        synthesis: synthesis.text,
        usage,
    };
    emit(&out, &human, opts.json)
}
