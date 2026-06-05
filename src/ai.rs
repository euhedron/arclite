use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// Token usage and cost for one synthesis call.
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// `None` when the CLI omitted cost — surfaced as "unknown", never a misleading $0.00.
    pub cost_usd: Option<f64>,
}

/// A synthesis result: the model's text plus what it cost.
#[derive(Debug, Clone, Serialize)]
pub struct Synthesis {
    pub text: String,
    pub usage: Usage,
}

/// A zero-cost prompt-size estimate (rough heuristic: ~4 chars per token).
#[derive(Debug, Clone, Serialize)]
pub struct Estimate {
    pub chars: usize,
    pub approx_tokens: usize,
}

/// Rough chars-per-token ratio for the zero-cost prompt estimate — approximate, and only a
/// pre-spend gauge; the real, billed token counts come back from the CLI after the call.
const CHARS_PER_TOKEN: usize = 4;

#[must_use]
pub fn estimate(prompt: &str) -> Estimate {
    let chars = prompt.chars().count();
    Estimate {
        chars,
        approx_tokens: chars.div_ceil(CHARS_PER_TOKEN),
    }
}

/// Build a [`Command`] for an external program. On Windows, npm-style `.cmd`
/// shims (e.g. `claude`) can't be launched by `Command::new` directly, so wrap
/// the call in `cmd /C`. Shared by `doctor`'s probe and [`synthesize`] so the
/// two never disagree about whether a tool is present.
pub fn command(program: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", program]);
        c
    } else {
        Command::new(program)
    }
}

// The subset of `claude -p --output-format json` we read.
#[derive(Deserialize)]
struct ClaudeJson {
    result: Option<String>,
    is_error: Option<bool>,
    total_cost_usd: Option<f64>,
    usage: Option<ClaudeUsage>,
    model: Option<String>,
}

#[derive(Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
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
    let usage = parsed.usage.unwrap_or_default();
    Ok(Synthesis {
        text,
        usage: Usage {
            model: parsed.model.unwrap_or_else(|| model.to_owned()),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cost_usd: parsed.total_cost_usd,
        },
    })
}

/// Run a synthesis through the Claude Code CLI with a deliberately minimal,
/// controlled context: an explicit model, no inherited MCP servers
/// (`--strict-mcp-config`), a neutral working directory (so the target repo's
/// CLAUDE.md is not auto-loaded), and the prompt passed over stdin (avoiding
/// shell-quoting pitfalls). Costs real tokens.
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
            .context("could not open claude's stdin")?;
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
