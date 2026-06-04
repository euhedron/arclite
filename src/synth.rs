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

/// Per-file char cap and total char budget across all `--include` paths. Sized for
/// *auditing* — whole source files should fit — while staying bounded; truncation is
/// always surfaced in the run's sources. Evolve as signals warrant.
const INCLUDE_FILE_CAP: usize = 32_000;
const INCLUDE_TOTAL_BUDGET: usize = 256_000;

/// Caps for the auto-included README and manifest bodies — lighter context than
/// `--include`d files (which exist to be read in full).
const README_CAP: usize = 4_000;
const MANIFEST_CAP: usize = 2_000;

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

/// A file's text capped to a budget: the (possibly truncated) body, the original
/// char count, and whether it was cut — so any truncation is reportable, not silent.
pub struct Capped {
    pub body: String,
    pub original_chars: usize,
    pub truncated: bool,
}

/// Read a file as text, capped at `max_chars` (char-safe), retaining enough to report
/// exactly what (if anything) was cut.
pub fn read_capped(path: &Path, max_chars: usize) -> Option<Capped> {
    let text = std::fs::read_to_string(path).ok()?;
    let original_chars = text.chars().count();
    if original_chars > max_chars {
        Some(Capped {
            body: format!(
                "{}\n…[truncated]",
                text.chars().take(max_chars).collect::<String>()
            ),
            original_chars,
            truncated: true,
        })
    } else {
        Some(Capped {
            body: text,
            original_chars,
            truncated: false,
        })
    }
}

/// A `sources` label for a capped file, making truncation explicit (which + by how much).
fn source_label(name: impl std::fmt::Display, cap: &Capped, max_chars: usize) -> String {
    if cap.truncated {
        format!(
            "{name} ({} chars, truncated to {max_chars})",
            cap.original_chars
        )
    } else {
        format!("{name} ({} chars)", cap.original_chars)
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
                Some(cap) => {
                    used += cap.body.chars().count();
                    included += 1;
                    sources.push(source_label(file.display(), &cap, INCLUDE_FILE_CAP));
                    ctx.push_str(&format!("\n{}:\n{}\n", file.display(), cap.body));
                }
                None if !is_dir => {
                    sources.push(format!("{} (unreadable — skipped)", file.display()));
                }
                None => {}
            }
        }
        if is_dir && included < total {
            sources.push(format!(
                "…{included} of {total} files under {} included (rest skipped: {}k-char budget reached or unreadable)",
                path.display(),
                INCLUDE_TOTAL_BUDGET / 1000
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

/// Assembled repo context plus a record of every source and what was excluded.
pub struct Context {
    pub text: String,
    pub sources: Vec<String>,
    pub excluded: Vec<String>,
}

/// Assemble the standard repo context shared by every synthesis command: the scan
/// summary, README + manifest bodies, any `--include`d files/dirs, and rules —
/// tracking each source (and what's excluded by default) for the run report. The
/// commands differ only in the prompt they wrap around this, never in grounding.
pub fn gather_context(
    path: &Path,
    includes: &[PathBuf],
    rules_dir: Option<&Path>,
) -> anyhow::Result<Context> {
    let report = crate::commands::inspect::gather(path)?;
    let root = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());

    let mut text = format!(
        "Repository scan (JSON):\n{}\n",
        serde_json::to_string_pretty(&report)?
    );
    let mut sources = vec!["repository scan".to_owned()];

    if let Some(cap) = read_capped(&root.join("README.md"), README_CAP) {
        sources.push(source_label("README.md", &cap, README_CAP));
        text.push_str(&format!("\nREADME:\n{}\n", cap.body));
    }
    for name in &report.manifests {
        if let Some(cap) = read_capped(&root.join(name), MANIFEST_CAP) {
            sources.push(source_label(name, &cap, MANIFEST_CAP));
            text.push_str(&format!("\n{name}:\n{}\n", cap.body));
        }
    }
    text.push_str(&gather_includes(includes, &mut sources));
    text.push_str(&gather_rules(rules_dir, &mut sources)?);

    let excluded = if includes.is_empty() {
        vec!["the repo's source files (--include <path> to add)".to_owned()]
    } else {
        Vec::new()
    };

    Ok(Context {
        text,
        sources,
        excluded,
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

    let synthesis = ai::synthesize(prompt, model, opts.allowed_tools)?;
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
