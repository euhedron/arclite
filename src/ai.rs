use std::io::Write;
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
    pub cost_usd: f64,
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

#[must_use]
pub fn estimate(prompt: &str) -> Estimate {
    let chars = prompt.chars().count();
    Estimate {
        chars,
        approx_tokens: chars.div_ceil(4),
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
            cost_usd: parsed.total_cost_usd.unwrap_or(0.0),
        },
    })
}

/// Run a synthesis through the Claude Code CLI with a deliberately minimal,
/// controlled context: an explicit model, no inherited MCP servers
/// (`--strict-mcp-config`), a neutral working directory (so the target repo's
/// CLAUDE.md is not auto-loaded), and the prompt passed over stdin (avoiding
/// shell-quoting pitfalls). Costs real tokens.
pub fn synthesize(prompt: &str, model: &str) -> anyhow::Result<Synthesis> {
    // `claude` is an npm shim; on Windows it must be invoked via cmd.
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "claude"]);
        c
    } else {
        Command::new("claude")
    };
    cmd.args([
        "-p",
        "--output-format",
        "json",
        "--model",
        model,
        "--strict-mcp-config",
    ])
    .current_dir(std::env::temp_dir())
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

#[cfg(test)]
mod tests {
    use super::*;

    // Real payload shape from a `claude -p ... --output-format json` probe.
    const SAMPLE: &str = r#"{"type":"result","is_error":false,"result":"ok","total_cost_usd":0.18,"model":"some-model","usage":{"input_tokens":1947,"output_tokens":4,"cache_creation_input_tokens":27378,"cache_read_input_tokens":0}}"#;

    #[test]
    fn parses_text_and_usage() {
        let s = parse_result(SAMPLE, "requested-model").unwrap();
        assert_eq!(s.text, "ok");
        assert_eq!(s.usage.input_tokens, 1947);
        assert_eq!(s.usage.cache_creation_input_tokens, 27378);
        assert_eq!(s.usage.model, "some-model");
        assert!((s.usage.cost_usd - 0.18).abs() < 1e-9);
    }

    #[test]
    fn falls_back_to_requested_model_label() {
        let json = r#"{"is_error":false,"result":"hi","total_cost_usd":0.0,"usage":{}}"#;
        let s = parse_result(json, "requested-model").unwrap();
        assert_eq!(s.usage.model, "requested-model");
    }

    #[test]
    fn surfaces_claude_errors() {
        assert!(parse_result(r#"{"is_error":true,"result":"boom"}"#, "m").is_err());
    }

    #[test]
    fn estimate_is_chars_over_four() {
        let e = estimate("abcdefgh");
        assert_eq!(e.chars, 8);
        assert_eq!(e.approx_tokens, 2);
    }
}
