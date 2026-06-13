use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// Token usage and cost for one synthesis call — ground truth from the CLI's response. `cost_usd` is
/// `Some` when the CLI returns an authoritative dollar cost (claude), and `None` when the backend
/// reports token usage but no cost (codex reports tokens only — no fabricated estimate).
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cost_usd: Option<f64>,
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
            cost_usd: Some(cost_usd),
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

/// A synthesis backend — a headless agent CLI arclite drives. It translates a backend-neutral
/// [`Request`] into that CLI's own invocation, folds the CLI's streamed events into the live-progress
/// marker, and parses the result into a [`Synthesis`]. The synthesis logic lives behind this trait,
/// rather than in one CLI-specific function, so a second backend (Codex is next) slots in without the
/// rest of arclite knowing which CLI ran.
pub trait Backend {
    /// Run one synthesis, streaming live progress into `progress`. Costs real tokens.
    fn synthesize(
        &self,
        req: &Request,
        progress: Option<crate::runs::Active>,
    ) -> anyhow::Result<Synthesis>;
}

/// Select a synthesis backend by name — the single home of the known backends and their wording.
pub fn backend(name: &str) -> anyhow::Result<Box<dyn Backend>> {
    match name {
        "claude" => Ok(Box::new(ClaudeBackend)),
        "codex" => Ok(Box::new(CodexBackend)),
        other => bail!("unknown backend `{other}` (known: claude, codex)"),
    }
}

/// The Claude Code CLI backend — `claude -p` with a controlled, isolated context: an explicit model,
/// no inherited MCP servers (`--strict-mcp-config`), and — unless `ambient_memory` is set — no ambient
/// memory, with the prompt passed over stdin (avoiding shell-quoting pitfalls). So by default the
/// sources arclite reports are authoritative, modulo Claude Code's own fixed base (date, env, tools).
pub struct ClaudeBackend;

impl Backend for ClaudeBackend {
    fn synthesize(
        &self,
        req: &Request,
        progress: Option<crate::runs::Active>,
    ) -> anyhow::Result<Synthesis> {
        synthesize_claude(req, progress)
    }
}

/// Drive `claude -p` for one [`Request`] — the [`ClaudeBackend`] implementation. Costs real tokens.
fn synthesize_claude(
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

/// Reasoning effort arclite requests of codex — specified explicitly (not read from the user's
/// `~/.codex/config.toml`) so a run is self-contained, via `-c model_reasoning_effort`. The highest
/// tier, matching the audit/critique role where judgment quality matters more than latency.
const CODEX_REASONING_EFFORT: &str = "xhigh";

/// The Codex CLI backend — `codex exec` with a read-only sandbox and a JSON event stream, the second
/// [`Backend`]. Codex reports token usage but no dollar cost, so the [`Usage`]'s `cost_usd` is `None`;
/// `--output-schema` takes a file path (claude takes the schema inline), so the request's schema is
/// materialized to a per-run temp file; and the final structured result is read from the `-o`
/// artifact, not scraped from the event stream (in `--json` mode stdout is events, not the answer).
pub struct CodexBackend;

impl Backend for CodexBackend {
    fn synthesize(
        &self,
        req: &Request,
        progress: Option<crate::runs::Active>,
    ) -> anyhow::Result<Synthesis> {
        synthesize_codex(req, progress)
    }
}

/// codex's `turn.completed.usage` token fields — tokens only, no dollar cost.
#[derive(Deserialize)]
struct CodexUsage {
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    reasoning_output_tokens: u64,
}

/// A per-run temp directory for codex's file-based `--output-schema`/`-o`, removed on drop. Unique
/// per call so concurrent `--runs N` codex runs can't collide on the schema/output files.
struct CodexTemp(PathBuf);

impl CodexTemp {
    fn new() -> anyhow::Result<Self> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("arclite-codex-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create codex temp dir {}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for CodexTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Drive `codex exec` for one [`Request`] — the [`CodexBackend`] implementation. Costs real tokens.
fn synthesize_codex(
    req: &Request,
    mut progress: Option<crate::runs::Active>,
) -> anyhow::Result<Synthesis> {
    let work = CodexTemp::new()?;
    let out_path = work.0.join("out.txt");
    let mut cmd = command("codex");
    cmd.arg("exec")
        .arg("--json")
        .arg("--sandbox")
        .arg("read-only") // arclite never has codex mutate the repo
        .arg("--skip-git-repo-check") // arclite points at any dir, not necessarily a git repo
        .arg("--model")
        .arg(req.model)
        .arg("-c")
        .arg(format!("model_reasoning_effort={CODEX_REASONING_EFFORT}"))
        .arg("--cd")
        .arg(req.dir)
        .arg("-o")
        .arg(&out_path);
    // Structured output: codex validates the final message against a schema *file* (claude takes it
    // inline), so materialize the request's schema to the run's temp dir.
    if let Some(schema) = req.json_schema {
        let schema_path = work.0.join("schema.json");
        std::fs::write(&schema_path, schema)
            .with_context(|| format!("cannot write codex schema to {}", schema_path.display()))?;
        cmd.arg("--output-schema").arg(&schema_path);
    }
    // req.max_budget_usd: codex exec has no budget cap — surfaced as claude-only by the caller, not here.
    // req.allowed_tools: codex runs read-only with no granted tools. req.ambient_memory: AGENTS.md
    // isolation (codex's CLAUDE_CODE_DISABLE_* analog) is still to be wired.
    cmd.current_dir(std::env::temp_dir()) // neutral cwd; --cd sets the agent's working root
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .context("failed to launch `codex` — is the Codex CLI installed and on PATH?")?;
    {
        let mut stdin = child.stdin.take().expect("stdin was configured as piped");
        stdin.write_all(req.prompt.as_bytes())?;
    } // dropping stdin closes it

    // Parse the JSONL event stream: fold agent-message items + completed turns into live progress and
    // keep the final usage. The structured result itself comes from the `-o` artifact below.
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let mut usage: Option<CodexUsage> = None;
    let mut agent_text = String::new();
    let mut failure: Option<String> = None;
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue; // non-JSON noise — skip
        };
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("turn.completed") => {
                usage = event
                    .get("usage")
                    .and_then(|u| serde_json::from_value(u.clone()).ok());
                if let Some(p) = progress.as_mut() {
                    p.record_turn(0); // codex tool-call accounting is coarser than claude's; turns only
                }
            }
            Some("item.completed") => {
                if let Some(item) = event.get("item")
                    && item.get("type").and_then(serde_json::Value::as_str) == Some("agent_message")
                    && let Some(text) = item.get("text").and_then(serde_json::Value::as_str)
                {
                    agent_text = text.to_owned();
                    if let Some(p) = progress.as_mut() {
                        p.record_text(text.chars().count() as u64);
                    }
                }
            }
            Some("turn.failed") | Some("error") => {
                failure = Some(
                    event
                        .pointer("/error/message")
                        .or_else(|| event.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("codex reported an error")
                        .to_owned(),
                );
            }
            _ => {}
        }
    }
    let mut stderr = String::new();
    let _ = child
        .stderr
        .take()
        .expect("stderr was configured as piped")
        .read_to_string(&mut stderr);
    let status = child.wait()?;
    if let Some(msg) = failure {
        bail!("codex reported an error: {msg}");
    }
    if !status.success() {
        bail!("codex exited with {}: {}", status, stderr.trim());
    }
    let usage = usage.context("codex produced no `turn.completed` usage event")?;
    // The final structured result is the `-o` artifact (clean + schema-valid); fall back to the last
    // agent message if `-o` wasn't written.
    let text = crate::read_optional(&out_path)?
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(agent_text);
    let structured = if req.json_schema.is_some() {
        Some(
            serde_json::from_str(text.trim())
                .context("codex did not return the expected JSON for the requested schema")?,
        )
    } else {
        None
    };
    Ok(Synthesis {
        text,
        usage: Usage {
            // Codex doesn't echo a per-model id in its events, so the reported model is the requested
            // one (unlike claude, which resolves it from the response's per-model usage).
            model: req.model.to_owned(),
            input_tokens: usage.input_tokens,
            // Codex separates reasoning tokens; fold them into output for an honest total-generated count.
            output_tokens: usage.output_tokens + usage.reasoning_output_tokens,
            cache_creation_input_tokens: 0, // codex has no cache-creation concept, only cached reads
            cache_read_input_tokens: usage.cached_input_tokens,
            cost_usd: None, // codex reports tokens only — no fabricated dollar cost
        },
        structured,
    })
}

// AI-output handling (parse_result) and the prompt estimate are exercised by
// using `summarize` — its cost/usage output makes any breakage immediately
// apparent — rather than via unit tests.
