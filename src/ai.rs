use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// Token usage and cost for one synthesis call — ground truth from the CLI's response.
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cost_usd: f64,
}

/// A synthesis result: the model's text plus what it cost.
#[derive(Debug, Clone, Serialize)]
pub struct Synthesis {
    pub text: String,
    pub usage: Usage,
    /// Schema-validated structured output — present only when a `--json-schema` was requested
    /// (i.e. `--structured`). Read this for the typed result instead of parsing `text`.
    pub structured: Option<serde_json::Value>,
}

/// A zero-cost prompt-size estimate (rough heuristic: ~4 chars per token).
#[derive(Debug, Clone, Serialize)]
pub struct Estimate {
    pub chars: usize,
    pub approx_tokens: usize,
}

/// Rough chars-per-token ratio for the zero-cost prompt estimate. Deliberately approximate and never
/// refreshed: it is only a pre-spend gauge — the real, billed token counts come back from the CLI.
const CHARS_PER_TOKEN: usize = 4;

#[must_use]
pub fn estimate(prompt: &str) -> Estimate {
    let chars = prompt.chars().count();
    Estimate {
        chars,
        approx_tokens: chars.div_ceil(CHARS_PER_TOKEN),
    }
}

/// Build a [`Command`] for an external program, resolved to a directly-spawnable executable.
/// `which` finds it on `PATH` (respecting Windows `PATHEXT`) — but on Windows that lands on the npm
/// `.cmd` shim, which forwards args through batch `%*`, corrupting quote-heavy args like an inline
/// `--json-schema` payload (Rust's `.cmd` quoting does not save them — confirmed empirically). So
/// when `which` returns such a shim, [`shim_target`] resolves the real `.exe` it runs and we spawn
/// that directly: no shell/batch re-parse, so std's standard argv quoting holds. Falls back to the
/// bare name (surfacing a normal "not found" error). Shared by `doctor`'s probe and [`synthesize`].
pub fn command(program: &str) -> Command {
    let exe = which::which(program)
        .ok()
        .map(|resolved| shim_target(&resolved).unwrap_or(resolved));
    match exe {
        Some(path) => Command::new(path),
        None => Command::new(program),
    }
}

/// If `path` is an npm-style `.cmd` shim, return the `.exe` it actually invokes — resolving the
/// shim's `%dp0%` (= its own directory) placeholder. `None` for non-shims or if no `.exe` resolves,
/// so callers fall back to the original path.
fn shim_target(path: &Path) -> Option<PathBuf> {
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd"))
    {
        return None;
    }
    let body = std::fs::read_to_string(path).ok()?;
    let dir = path.parent()?;
    // npm shim runs e.g. `"%dp0%\node_modules\…\claude.exe"   %*`; pull that quoted .exe out.
    body.split('"')
        .filter(|tok| tok.to_ascii_lowercase().ends_with(".exe"))
        .find_map(|tok| {
            let rel = tok
                .trim_start_matches("%dp0%")
                .trim_start_matches(['\\', '/']);
            let candidate = dir.join(rel);
            candidate.is_file().then_some(candidate)
        })
}

// The subset of `claude -p --output-format json` we read.
#[derive(Deserialize)]
struct ClaudeJson {
    result: Option<String>,
    is_error: Option<bool>,
    total_cost_usd: Option<f64>,
    usage: Option<ClaudeUsage>,
    model: Option<String>,
    /// Present when `--json-schema` was passed: the validated, typed result.
    structured_output: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

/// Parse the Claude CLI JSON payload into a [`Synthesis`]. `model` is the
/// requested model, used as a fallback label if the payload omits it.
pub fn parse_result(json: &str, model: &str) -> anyhow::Result<Synthesis> {
    let parsed: ClaudeJson =
        serde_json::from_str(json).context("claude did not return the expected JSON")?;
    if parsed.is_error.unwrap_or(false) {
        bail!(
            "claude reported an error: {}",
            parsed.result.unwrap_or_default()
        );
    }
    let text = parsed.result.context("claude JSON had no `result` field")?;
    // usage and cost are part of a successful response's contract; if the CLI omits them, error
    // loudly rather than fabricate zeros that would read as genuine zero spend.
    let usage = parsed.usage.context("claude JSON had no `usage` field")?;
    let cost_usd = parsed
        .total_cost_usd
        .context("claude JSON had no `total_cost_usd` field")?;
    Ok(Synthesis {
        text,
        usage: Usage {
            model: parsed.model.unwrap_or_else(|| model.to_owned()),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cost_usd,
        },
        structured: parsed.structured_output,
    })
}

/// Run a synthesis through the Claude Code CLI with a deliberately minimal, controlled context: an
/// explicit model, no inherited MCP servers (`--strict-mcp-config`), and — unless `ambient_memory`
/// is set — no ambient memory, with the prompt passed over stdin (avoiding shell-quoting pitfalls).
/// So by default the sources arclite reports are authoritative, modulo Claude Code's own fixed base
/// (date, env, tools). Costs real tokens.
///
/// `allowed_tools` is a major cost lever — Claude Code's tool schemas dominate
/// the context. An empty slice restricts to no tools (cheapest, ~10x less than
/// the full default — right for pure text synthesis); a non-empty slice allows
/// exactly those tools, and grants them read access to `dir` (the repo) via
/// `--add-dir`, since the working directory is neutral.
pub fn synthesize(
    prompt: &str,
    model: &str,
    allowed_tools: &[String],
    dir: &Path,
    ambient_memory: bool,
    json_schema: Option<&str>,
) -> anyhow::Result<Synthesis> {
    let mut cmd = command("claude");
    cmd.args([
        "-p",
        "--output-format",
        "json",
        "--model",
        model,
        "--strict-mcp-config",
    ]);
    cmd.arg("--allowedTools");
    if allowed_tools.is_empty() {
        cmd.arg(""); // allowlist of none → no tool schemas loaded (minimal context)
    } else {
        cmd.args(allowed_tools);
        // cwd is neutral (below), so grant the allowed tools read access to the repo.
        cmd.arg("--add-dir").arg(dir);
    }
    // Structured output (`--structured`): the result returns as a schema-validated `structured_output`
    // object, never scraped from prose. The which-resolved direct spawn (see `command`) passes this
    // quote-heavy arg through intact, where the old `cmd /C` path would have mangled it.
    if let Some(schema) = json_schema {
        cmd.arg("--json-schema").arg(schema);
    }
    // Disable auto-loading of user/project CLAUDE.md + auto-memory. A neutral cwd alone does NOT stop
    // it — the user-level ~/.claude/CLAUDE.md loads regardless of cwd. This affects only context
    // loading, not the separate credential store, so auth is unaffected; `--ambient-memory` opts in.
    if !ambient_memory {
        cmd.env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1");
        cmd.env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
    }
    cmd.current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .context("failed to launch `claude` — is the Claude Code CLI installed and on PATH?")?;
    {
        let mut stdin = child
            .stdin
            .take()
            .expect("stdin was configured as piped");
        stdin.write_all(prompt.as_bytes())?;
    } // dropping stdin closes it, signalling end-of-input

    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "claude exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_result(&stdout, model)
}

// AI-output handling (parse_result) and the prompt estimate are exercised by
// using `summarize` — its cost/usage output makes any breakage immediately
// apparent — rather than via unit tests. See the project's testing note.
