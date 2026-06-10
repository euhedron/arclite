use std::io::{BufRead, Read, Write};
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

/// A zero-cost prompt-size estimate: the prompt's char count and an approximate token count.
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
/// bare name (surfacing a normal "not found" error). Shared by every external-process call arclite makes.
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

// The subset of the CLI's final `result` payload we read (the last event of
// `--output-format stream-json`, carrying what `--output-format json` would return whole).
#[derive(Deserialize)]
struct ClaudeJson {
    result: Option<String>,
    is_error: Option<bool>,
    /// Names the failure on an error payload (e.g. `error_max_budget_usd`), where `result` is absent.
    subtype: Option<String>,
    total_cost_usd: Option<f64>,
    usage: Option<ClaudeUsage>,
    /// Per-model usage, one entry per model that ran — the payload's only model identification
    /// (there is no top-level model id — confirmed by exercise), and so the ground truth the
    /// reported model resolves from.
    #[serde(rename = "modelUsage", default)]
    model_usage: std::collections::BTreeMap<String, PerModelUsage>,
    /// Present when `--json-schema` was passed: the validated, typed result.
    structured_output: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct PerModelUsage {
    #[serde(rename = "outputTokens", default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

/// Parse the Claude CLI JSON payload into a [`Synthesis`]. The model reported is resolved from the
/// payload's per-model usage — the models that actually ran — never echoed from the request, so a
/// substitution can't mislabel the run.
pub fn parse_result(json: &str) -> anyhow::Result<Synthesis> {
    let parsed: ClaudeJson =
        serde_json::from_str(json).context("claude did not return the expected JSON")?;
    if parsed.is_error.unwrap_or(false) {
        // An error payload carries no `result` text (confirmed on a tripped --max-budget-usd run);
        // its `subtype` names the failure instead.
        let what = parsed
            .result
            .filter(|r| !r.is_empty())
            .or(parsed.subtype)
            .unwrap_or_else(|| "no detail in the payload".to_owned());
        bail!("claude reported an error: {what}");
    }
    let text = parsed.result.context("claude JSON had no `result` field")?;
    // usage and cost are part of a successful response's contract; if the CLI omits them, error
    // loudly rather than fabricate zeros that would read as genuine zero spend.
    let usage = parsed.usage.context("claude JSON had no `usage` field")?;
    let cost_usd = parsed
        .total_cost_usd
        .context("claude JSON had no `total_cost_usd` field")?;
    // The synthesis model is the modelUsage entry that produced the output — the one with the most
    // output tokens (the CLI's internal auxiliary models make comparatively tiny calls). Like usage
    // and cost, an absent modelUsage errors loudly rather than substituting a plausible label.
    let model = parsed
        .model_usage
        .iter()
        .max_by_key(|(_, usage)| usage.output_tokens)
        .map(|(id, _)| id.clone())
        .context("claude JSON had no `modelUsage` entries")?;
    Ok(Synthesis {
        text,
        usage: Usage {
            model,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cost_usd,
        },
        structured: parsed.structured_output,
    })
}

/// One synthesis call's run-shaping configuration — a struct, so a new parameter is a field rather
/// than another argument every call site must repeat in order.
pub struct Request<'a> {
    pub prompt: &'a str,
    pub model: &'a str,
    pub allowed_tools: &'a [String],
    /// Repository root, granted to allowed tools via `--add-dir` (the working directory is neutral).
    pub dir: &'a Path,
    pub ambient_memory: bool,
    pub json_schema: Option<&'a str>,
    pub max_budget_usd: Option<f64>,
}

/// Run a synthesis through the Claude Code CLI with a controlled, isolated context: an
/// explicit model, no inherited MCP servers (`--strict-mcp-config`), and — unless `ambient_memory`
/// is set — no ambient memory, with the prompt passed over stdin (avoiding shell-quoting pitfalls).
/// So by default the sources arclite reports are authoritative, modulo Claude Code's own fixed base
/// (date, env, tools). Costs real tokens.
pub fn synthesize(
    req: &Request,
    mut progress: Option<crate::runs::Active>,
) -> anyhow::Result<Synthesis> {
    let mut cmd = command("claude");
    // stream-json + --verbose + partial messages: stream events as the run proceeds — `assistant`
    // events at turn boundaries plus fine-grained `content_block_delta`s — so live stats update
    // continuously; the final `result` event carries the same payload `--output-format json` would.
    cmd.args([
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--model",
        req.model,
        "--strict-mcp-config",
    ]);
    cmd.arg("--allowedTools");
    if req.allowed_tools.is_empty() {
        cmd.arg(""); // allowlist of none → no tool schemas loaded (minimal context)
    } else {
        cmd.args(req.allowed_tools);
        // cwd is neutral (below), so grant the allowed tools read access to the repo.
        cmd.arg("--add-dir").arg(req.dir);
    }
    // Hard cost cap — the CLI enforces it and a tripped run errors with subtype
    // `error_max_budget_usd`; the enforcement semantics live on the `--max-budget-usd` flag doc.
    if let Some(cap) = req.max_budget_usd {
        cmd.arg("--max-budget-usd").arg(cap.to_string());
    }
    // Structured output (`--structured`): the result returns as a schema-validated `structured_output`
    // object, never scraped from prose.
    if let Some(schema) = req.json_schema {
        cmd.arg("--json-schema").arg(schema);
    }
    // Disable auto-loading of user/project CLAUDE.md + auto-memory. A neutral cwd alone does NOT stop
    // it — the user-level ~/.claude/CLAUDE.md loads regardless of cwd. This affects only context
    // loading, not the separate credential store, so auth is unaffected; `--ambient-memory` opts in.
    if !req.ambient_memory {
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
        stdin.write_all(req.prompt.as_bytes())?;
    } // dropping stdin closes it, signalling end-of-input

    // Read the event stream line-by-line as it arrives: fold each `assistant` turn into live stats
    // (via `on_turn`) and keep the final `result` event — the payload `parse_result` understands.
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let mut result_line: Option<String> = None;
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue; // non-JSON noise (e.g. a stdin warning) — skip
        };
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("stream_event") => {
                // A content_block_delta's text is the streamed output; its length is the continuous
                // live signal. Only text_delta carries a string `text` (tool-input/thinking deltas
                // don't), so probing that field both filters to it and extracts it in one step.
                if let Some(p) = progress.as_mut()
                    && event.pointer("/event/type").and_then(serde_json::Value::as_str)
                        == Some("content_block_delta")
                    && let Some(text) = event
                        .pointer("/event/delta/text")
                        .and_then(serde_json::Value::as_str)
                {
                    p.record_text(text.chars().count() as u64);
                }
            }
            Some("assistant") => {
                if let Some(p) = progress.as_mut() {
                    let tool_calls = event
                        .pointer("/message/content")
                        .and_then(serde_json::Value::as_array)
                        .map_or(0, |blocks| {
                            blocks
                                .iter()
                                .filter(|b| {
                                    b.get("type").and_then(serde_json::Value::as_str)
                                        == Some("tool_use")
                                })
                                .count() as u64
                        });
                    p.record_turn(tool_calls);
                }
            }
            Some("result") => result_line = Some(line),
            _ => {}
        }
    }
    // stderr is small for `claude -p` (warnings), so reading it after stdout drains won't deadlock.
    let mut stderr = String::new();
    let _ = child
        .stderr
        .take()
        .expect("stderr was configured as piped")
        .read_to_string(&mut stderr);
    let status = child.wait()?;
    // A failed run usually still emits a `result` error event (e.g. a tripped --max-budget-usd cap:
    // is_error + subtype) — parse that for the real failure rather than reporting a bare exit code.
    let result_line = match (result_line, status.success()) {
        (Some(line), _) => line,
        (None, false) => bail!("claude exited with {}: {}", status, stderr.trim()),
        (None, true) => bail!("claude produced no `result` event"),
    };
    let synthesis = parse_result(&result_line)?;
    if !status.success() {
        // The payload parsed as a success yet the process failed — surface the contradiction.
        bail!("claude exited with {} despite a success result", status);
    }
    Ok(synthesis)
}

// AI-output handling (parse_result) and the prompt estimate are exercised by
// using `summarize` — its cost/usage output makes any breakage immediately
// apparent — rather than via unit tests.
