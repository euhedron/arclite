use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

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
    /// An agent-reported failure (e.g. a tripped `--max-budget-usd` cap → `error_max_budget_usd`): the
    /// run *ran and spent* — so `usage` holds the real, billed spend — but did not complete. Carried as
    /// a value, not an `Err`, so [`crate::synth::run`] still logs the spend instead of losing it on the
    /// error path. `None` is a normal completion.
    pub error: Option<String>,
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
/// that directly: no shell/batch re-parse, so std's standard argv quoting holds. A program genuinely
/// *not on* `PATH` falls back to the bare name, so the spawn surfaces a normal "not found"; a real
/// PATH-resolution failure (not mere absence) is surfaced rather than disguised as "not found", and
/// the fallible return also propagates [`shim_target`]'s error. Shared by every external-process call.
pub fn command(program: &str) -> anyhow::Result<Command> {
    let exe = match which::which(program) {
        Ok(resolved) => Some(shim_target(&resolved)?.unwrap_or(resolved)),
        // Genuinely not on PATH: fall back to the bare name so the spawn surfaces a normal "not
        // found". Any *other* resolution failure is real — surface it rather than disguise it as
        // "not found" at spawn (mirrors doctor's probe, which separates not-found from other errors).
        Err(which::Error::CannotFindBinaryPath) => None,
        Err(e) => return Err(e).context(format!("resolving `{program}` on PATH")),
    };
    Ok(match exe {
        Some(path) => Command::new(path),
        None => Command::new(program),
    })
}

/// If `path` is an npm-style `.cmd` shim, return the `.exe` it actually invokes — resolving the
/// shim's `%dp0%` (= its own directory) placeholder. `Ok(None)` for a non-`.cmd`, or a `.cmd` whose
/// body names no resolvable `.exe` (callers fall back to the original path); `Err` if the shim
/// exists — `which` just resolved it — but can't be read, a genuine failure never swallowed into the
/// "not a shim" case (that read failure resurfacing as the arg-corruption bug is the whole point).
fn shim_target(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd"))
    {
        return Ok(None);
    }
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the shim {}", path.display()))?;
    let dir = path
        .parent()
        .expect("a which-resolved shim path always has a parent directory");
    // npm shim runs e.g. `"%dp0%\node_modules\…\claude.exe"   %*`; pull that quoted .exe out.
    Ok(body
        .split('"')
        .filter(|tok| tok.to_ascii_lowercase().ends_with(".exe"))
        .find_map(|tok| {
            let rel = tok
                .trim_start_matches("%dp0%")
                .trim_start_matches(['\\', '/']);
            let candidate = dir.join(rel);
            candidate.is_file().then_some(candidate)
        }))
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
    /// Human-readable failure detail on an error payload (e.g. "Reached maximum budget ($0.001)"),
    /// preferred over the bare `subtype` code when present.
    #[serde(default)]
    errors: Vec<String>,
}

#[derive(Deserialize)]
struct PerModelUsage {
    #[serde(rename = "inputTokens", default)]
    input_tokens: u64,
    #[serde(rename = "outputTokens", default)]
    output_tokens: u64,
    #[serde(rename = "cacheCreationInputTokens", default)]
    cache_creation_input_tokens: u64,
    #[serde(rename = "cacheReadInputTokens", default)]
    cache_read_input_tokens: u64,
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
pub fn parse_result(json: &str, requested_model: &str) -> anyhow::Result<Synthesis> {
    let parsed: ClaudeJson =
        serde_json::from_str(json).context("claude did not return the expected JSON")?;
    // The synthesis model is the modelUsage entry that produced the output — the one with the most
    // output tokens (the CLI's internal auxiliary models make comparatively tiny calls). Resolved once,
    // shared by the success and error paths.
    let model = parsed
        .model_usage
        .iter()
        .max_by_key(|(_, usage)| usage.output_tokens)
        .map(|(id, _)| id.clone());
    if parsed.is_error.unwrap_or(false) {
        // A run that ran and *spent* but did not complete (e.g. a tripped --max-budget-usd cap). On an
        // error payload the top-level `usage` block is zeroed (confirmed by exercise) while the real
        // tokens are in `modelUsage` and the real cost in `total_cost_usd` — so the honest usage sums
        // modelUsage rather than reading the zeros, and the failure is carried as a value (logged), not
        // bailed (which would lose the spend). The model falls back to the requested one only when the
        // payload named none (an error before any model ran).
        let usage = Usage {
            model: model.unwrap_or_else(|| requested_model.to_owned()),
            input_tokens: parsed.model_usage.values().map(|m| m.input_tokens).sum(),
            output_tokens: parsed.model_usage.values().map(|m| m.output_tokens).sum(),
            cache_creation_input_tokens: parsed
                .model_usage
                .values()
                .map(|m| m.cache_creation_input_tokens)
                .sum(),
            cache_read_input_tokens: parsed
                .model_usage
                .values()
                .map(|m| m.cache_read_input_tokens)
                .sum(),
            cost_usd: parsed.total_cost_usd,
        };
        // Prefer the human-readable `errors` detail; fall back to the `subtype` code, then a placeholder.
        let detail = if parsed.errors.is_empty() {
            parsed
                .subtype
                .unwrap_or_else(|| "no detail in the payload".to_owned())
        } else {
            parsed.errors.join("; ")
        };
        return Ok(Synthesis {
            text: String::new(),
            usage,
            structured: None,
            error: Some(detail),
        });
    }
    let text = parsed.result.context("claude JSON had no `result` field")?;
    // usage and cost are part of a successful response's contract; if the CLI omits them, error
    // loudly rather than fabricate zeros that would read as genuine zero spend.
    let usage = parsed.usage.context("claude JSON had no `usage` field")?;
    let cost_usd = parsed
        .total_cost_usd
        .context("claude JSON had no `total_cost_usd` field")?;
    let model = model.context("claude JSON had no `modelUsage` entries")?;
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
        error: None,
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
    /// Codex reasoning effort (`-c model_reasoning_effort`); `None` for backends that don't use it.
    pub reasoning_effort: Option<&'a str>,
}

/// arclite's default synthesis backend, used when neither `--backend` nor `defaults.backend` is set.
pub const DEFAULT_BACKEND: &str = "claude";

/// Constructs a backend instance — the factory half of a [`BACKENDS`] registry row.
type BackendFactory = fn() -> Box<dyn Backend>;

/// The known synthesis backends: each name paired with its `Backend` constructor — the one registry
/// `backend()` dispatches from and `known_backends()` lists, so adding a backend is a single row here,
/// not a name list and a `match` arm kept in lockstep (the factory is what a name list alone can't
/// encode). `doctor` probes, `validate_backend`, and error wording all derive from the name set.
const BACKENDS: &[(&str, BackendFactory)] = &[
    ("claude", || Box::new(ClaudeBackend)),
    ("codex", || Box::new(CodexBackend)),
];

/// The known backend names, derived from the [`BACKENDS`] registry.
pub(crate) fn known_backends() -> Vec<&'static str> {
    BACKENDS.iter().map(|(name, _)| *name).collect()
}

/// The claude backend's default model. Update when a newer model supersedes it; the run reports the
/// resolved id the response returns.
const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// The codex backend's default model — specified explicitly (not read from codex's own `config.toml`)
/// so a codex run is self-contained. Update as codex's lineup advances; the run reports the id used.
const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";

/// A synthesis backend — a headless agent CLI arclite drives. It translates a backend-neutral
/// [`Request`] into that CLI's own invocation, folds the CLI's streamed events into the live-progress
/// marker, and parses the result into a [`Synthesis`]. Per-backend policy — its default model, whether
/// it honors a native spend cap, which requested capabilities it can't — lives behind this trait too,
/// so the rest of arclite drives any backend without branching on which CLI it is.
pub trait Backend {
    /// Run one synthesis, streaming live progress into `progress`. Costs real tokens.
    fn synthesize(
        &self,
        req: &Request,
        progress: Option<crate::runs::Active>,
    ) -> anyhow::Result<Synthesis>;

    /// This backend's default model, when neither `--model` nor an applicable configured default is
    /// set. Per-backend because the backends' model families differ.
    fn default_model(&self) -> &'static str;

    /// This backend's configured default model from settings (`defaults.model` for claude,
    /// `defaults.codex_model` for codex), or `None` if unset. Required, so model resolution never
    /// branches on the backend name and a new backend must declare its own key rather than inherit one.
    fn configured_model<'s>(&self, settings: &'s Settings) -> Option<&'s str>;

    /// Resolve the run's model: an explicit `--model` wins; else this backend's configured default
    /// (its [`Backend::configured_model`]); else [`Backend::default_model`].
    fn resolve_model(&self, explicit: Option<&str>, configured: Option<&str>) -> String {
        explicit
            .map(str::to_owned)
            .or_else(|| configured.map(str::to_owned))
            .unwrap_or_else(|| self.default_model().to_owned())
    }

    /// Resolve the run's per-run spend cap from the explicit `--max-budget-usd` and the configured
    /// default. Default: both apply (the backend honors a native cap). A backend with none returns
    /// `None`, so neither an explicit cap (already refused by [`Backend::reject_unsupported`]) nor a
    /// default is mistaken for an enforced limit.
    fn resolve_budget(&self, explicit: Option<f64>, default: Option<f64>) -> Option<f64> {
        explicit.or(default)
    }

    /// Reject, before any spend, a requested capability this backend can't honor — surfaced as an
    /// error, never silently dropped. Default: honor everything. A backend overrides to refuse what it
    /// lacks; no backend is privileged — each declares its own limits here.
    fn reject_unsupported(
        &self,
        max_budget_usd: Option<f64>,
        allowed_tools: &[String],
    ) -> anyhow::Result<()> {
        let _ = (max_budget_usd, allowed_tools);
        Ok(())
    }

    /// The reasoning effort this backend runs at, given any configured value — surfaced in the report
    /// and applied to the call, because it shapes cost. Default: `None` (the backend has no such knob).
    /// A backend with one returns the effective value (the configured one, else its own default).
    fn reasoning_effort(&self, configured: Option<&str>) -> Option<String> {
        let _ = configured;
        None
    }
}

/// Select a synthesis backend by name — dispatched from the single [`BACKENDS`] registry.
pub fn backend(name: &str) -> anyhow::Result<Box<dyn Backend>> {
    BACKENDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, make)| make())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown backend `{name}` (known: {})",
                known_backends().join(", ")
            )
        })
}

/// The Claude Code CLI backend — `claude -p` with a controlled, isolated context: an explicit model,
/// no inherited MCP servers (`--strict-mcp-config`), and — unless `ambient_memory` is set — no ambient
/// memory, with the prompt passed over stdin (avoiding shell-quoting pitfalls). So by default the
/// sources arclite reports are authoritative, modulo Claude Code's own fixed base (date, env, tools).
pub struct ClaudeBackend;

impl Backend for ClaudeBackend {
    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    fn configured_model<'s>(&self, settings: &'s Settings) -> Option<&'s str> {
        settings.default_model.as_deref()
    }

    fn synthesize(
        &self,
        req: &Request,
        progress: Option<crate::runs::Active>,
    ) -> anyhow::Result<Synthesis> {
        synthesize_claude(req, progress)
    }
}

/// Spawn a configured agent-CLI `cmd`, feed it `prompt` on stdin, and fold its stdout JSONL event
/// stream line-by-line through `on_event` — `(the event's "type", the parsed event, the raw line)`,
/// non-JSON lines skipped — then return the exit status and captured stderr. The shared process-driving
/// scaffold: the backends differ only in how they build `cmd` and what they fold from each event, so
/// this plumbing lives here once and can't drift.
fn drive(
    mut cmd: Command,
    prompt: &str,
    launch_err: &'static str,
    mut on_event: impl FnMut(&str, &serde_json::Value, &str),
) -> anyhow::Result<(std::process::ExitStatus, String)> {
    cmd.current_dir(std::env::temp_dir()) // neutral cwd; the agent's working root is set per-backend
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context(launch_err)?;
    // Write the prompt on its own thread, concurrently with draining stdout/stderr below: writing it
    // all first (a large prompt on a small pipe buffer, notably Windows) deadlocks if the child emits
    // output before consuming stdin — its output pipe fills while we're blocked writing stdin. The
    // write result isn't propagated: a broken pipe just means the child exited early (e.g. a budget
    // rejection), and the run's outcome comes from the result event / stderr / exit status, not from
    // whether all stdin was written. Dropping stdin when the closure ends signals end-of-input.
    let mut stdin = child.stdin.take().expect("stdin was configured as piped");
    let prompt = prompt.to_owned();
    let stdin_writer = std::thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
    });
    // Drain stderr on its own thread, concurrently with the stdout stream below: a backend that fills
    // the stderr pipe buffer while we're still reading stdout would block writing it — and we'd never
    // reach a post-loop read — a deadlock. Joined after the child exits.
    let mut stderr_pipe = child.stderr.take().expect("stderr was configured as piped");
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr = String::new();
        let _ = stderr_pipe.read_to_string(&mut stderr);
        stderr
    });
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line?;
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue; // non-JSON noise (e.g. a stdin warning) — skip
        };
        let kind = event
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        on_event(kind, &event, &line);
    }
    let status = child.wait()?;
    let stderr = stderr_reader
        .join()
        .expect("the stderr reader thread panicked");
    stdin_writer
        .join()
        .expect("the stdin writer thread panicked");
    Ok((status, stderr))
}

/// Drive `claude -p` for one [`Request`] — the [`ClaudeBackend`] implementation. Costs real tokens.
fn synthesize_claude(
    req: &Request,
    mut progress: Option<crate::runs::Active>,
) -> anyhow::Result<Synthesis> {
    let mut cmd = command("claude")?;
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
    let mut result_line: Option<String> = None;
    let (status, stderr) = drive(
        cmd,
        req.prompt,
        "failed to launch `claude` — is the Claude Code CLI installed and on PATH?",
        |kind, event, raw| match kind {
            "stream_event" => {
                // A content_block_delta's text is the streamed output; its length is the continuous
                // live signal. Only text_delta carries a string `text` (tool-input/thinking deltas
                // don't), so probing that field both filters to it and extracts it in one step.
                if let Some(p) = progress.as_mut()
                    && event
                        .pointer("/event/type")
                        .and_then(serde_json::Value::as_str)
                        == Some("content_block_delta")
                    && let Some(text) = event
                        .pointer("/event/delta/text")
                        .and_then(serde_json::Value::as_str)
                {
                    p.record_text(text.chars().count() as u64);
                }
            }
            "assistant" => {
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
            "result" => result_line = Some(raw.to_owned()),
            _ => {}
        },
    )?;
    // A failed run usually still emits a `result` error event (e.g. a tripped --max-budget-usd cap:
    // is_error + subtype) — parse that for the real failure rather than reporting a bare exit code.
    let result_line = match (result_line, status.success()) {
        (Some(line), _) => line,
        (None, false) => bail!("claude exited with {}: {}", status, stderr.trim()),
        (None, true) => bail!("claude produced no `result` event"),
    };
    let synthesis = parse_result(&result_line, req.model)?;
    // A non-zero exit whose payload parsed as an error is the expected failed-run shape — the failure
    // is carried in `synthesis.error` (with its real usage) for logging. Only a non-zero exit that
    // parsed as a *success* is a genuine contradiction worth bailing on.
    if !status.success() && synthesis.error.is_none() {
        bail!("claude exited with {} despite a success result", status);
    }
    Ok(synthesis)
}

/// The codex backend's *default* reasoning effort, used when `defaults.codex_reasoning_effort` isn't
/// set — specified explicitly (not read from codex's `config.toml`) so a run is self-contained, and
/// surfaced in the run report since it shapes cost. The highest tier, matching the audit/critique role
/// where judgment quality matters more than latency.
const CODEX_REASONING_EFFORT: &str = "xhigh";

/// The reasoning-effort levels codex's `model_reasoning_effort` accepts. Update if codex's lineup
/// changes; [`CODEX_REASONING_EFFORT`] (the default) must be one of these.
const CODEX_REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];

/// Validate a configured / `config set` backend name against the known set — delegating to [`backend`],
/// the single authority — so a typo is rejected at set + load time, not only when a run tries to use it.
pub(crate) fn validate_backend(name: &str) -> anyhow::Result<()> {
    backend(name).map(|_| ())
}

/// Validate a configured / `config set` codex reasoning effort against [`CODEX_REASONING_EFFORTS`], so a
/// typo is rejected at set + load time rather than only when codex rejects it mid-run.
pub(crate) fn validate_reasoning_effort(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        CODEX_REASONING_EFFORTS.contains(&value),
        "invalid codex reasoning effort `{value}` (known: {})",
        CODEX_REASONING_EFFORTS.join(", ")
    );
    Ok(())
}

/// The Codex CLI backend — `codex exec` with a read-only sandbox and a JSON event stream, the second
/// [`Backend`]. Codex reports token usage but no dollar cost, so the [`Usage`]'s `cost_usd` is `None`;
/// `--output-schema` takes a file path (claude takes the schema inline), so the request's schema is
/// materialized to a per-run temp file; and the final structured result is read from the `-o`
/// artifact, not scraped from the event stream (in `--json` mode stdout is events, not the answer).
pub struct CodexBackend;

impl Backend for CodexBackend {
    fn default_model(&self) -> &'static str {
        DEFAULT_CODEX_MODEL
    }

    fn configured_model<'s>(&self, settings: &'s Settings) -> Option<&'s str> {
        settings.default_codex_model.as_deref()
    }

    /// codex exposes no native per-run spend cap, so none applies (an explicit one is refused below).
    fn resolve_budget(&self, _explicit: Option<f64>, _default: Option<f64>) -> Option<f64> {
        None
    }

    fn reject_unsupported(
        &self,
        max_budget_usd: Option<f64>,
        allowed_tools: &[String],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            max_budget_usd.is_none(),
            "--max-budget-usd requests a native per-run spend cap, which the codex backend has no equivalent for"
        );
        anyhow::ensure!(
            allowed_tools.is_empty(),
            "--allow-tool (a claude-style tool-name allowlist) isn't mapped onto codex's tool model (MCP + sandbox) yet"
        );
        Ok(())
    }

    /// codex bills by reasoning effort, so the effective value (configured, else the default) is
    /// surfaced and applied — never a hidden cost-shaping knob.
    fn reasoning_effort(&self, configured: Option<&str>) -> Option<String> {
        Some(configured.unwrap_or(CODEX_REASONING_EFFORT).to_owned())
    }

    fn synthesize(
        &self,
        req: &Request,
        progress: Option<crate::runs::Active>,
    ) -> anyhow::Result<Synthesis> {
        synthesize_codex(req, progress)
    }
}

/// codex's `turn.completed.usage` token fields — tokens only, no dollar cost. `Default` (all zeros)
/// is the honest shape for a run that failed before any turn completed: no usage was ever reported,
/// mirroring the claude error path's empty-`modelUsage` case.
#[derive(Deserialize, Default)]
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
        // Best-effort cleanup, surfaced rather than silent: a failed removal leaks a temp directory.
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            eprintln!(
                "arclite: couldn't remove the codex temp dir {} ({e}); it may be left behind",
                self.0.display()
            );
        }
    }
}

/// Drive `codex exec` for one [`Request`] — the [`CodexBackend`] implementation. Costs real tokens.
fn synthesize_codex(
    req: &Request,
    mut progress: Option<crate::runs::Active>,
) -> anyhow::Result<Synthesis> {
    let work = CodexTemp::new()?;
    let out_path = work.0.join("out.txt");
    let mut cmd = command("codex")?;
    cmd.arg("exec")
        .arg("--json")
        .arg("--sandbox")
        .arg("read-only") // arclite never has codex mutate the repo
        .arg("--skip-git-repo-check") // arclite points at any dir, not necessarily a git repo
        .arg("--ignore-user-config") // ignore ~/.codex/config.toml — arclite sets the run explicitly, so it's reproducible (auth.json is separate, so it still applies)
        .arg("--ignore-rules") // skip project .rules execpolicy (arclite runs read-only regardless)
        .arg("--model")
        .arg(req.model)
        .arg("-c")
        .arg("approval_policy=never") // a headless run must never pause for approval (not an exec flag, so set via config)
        .arg("--cd")
        .arg(req.dir)
        .arg("-o")
        .arg(&out_path);
    // Reasoning effort is cost-shaping, so the caller resolves + surfaces it (the codex backend always
    // supplies one via `reasoning_effort()`) and it's applied here, not hidden as a fixed default.
    if let Some(effort) = req.reasoning_effort {
        cmd.arg("-c")
            .arg(format!("model_reasoning_effort={effort}"));
    }
    // Isolate the repo's AGENTS.md (codex's ambient-memory analog) by default — embed 0 bytes of it —
    // mirroring claude's CLAUDE.md isolation; `--ambient-memory` opts into loading it.
    if !req.ambient_memory {
        cmd.arg("-c").arg("project_doc_max_bytes=0");
    }
    // Structured output: codex validates the final message against a schema *file* (claude takes it
    // inline), so materialize the request's schema to the run's temp dir.
    if let Some(schema) = req.json_schema {
        let schema_path = work.0.join("schema.json");
        std::fs::write(&schema_path, schema)
            .with_context(|| format!("cannot write codex schema to {}", schema_path.display()))?;
        cmd.arg("--output-schema").arg(&schema_path);
    }
    // codex runs read-only with no tools here (--sandbox read-only, no MCP configured).
    let mut usage_raw: Option<serde_json::Value> = None;
    let mut failure: Option<String> = None;
    let (status, stderr) = drive(
        cmd,
        req.prompt,
        "failed to launch `codex` — is the Codex CLI installed and on PATH?",
        |kind, event, _raw| match kind {
            "turn.completed" => {
                // Capture the raw usage object; parse it after the stream so a *malformed* usage
                // surfaces as a parse error rather than being swallowed to None and misreported as an
                // absent event (the honest ground-truth-usage contract — cf. parse_result's loud bails).
                usage_raw = event.get("usage").cloned();
                if let Some(p) = progress.as_mut() {
                    p.record_turn(0); // codex tool-call accounting is coarser than claude's; turns only
                }
            }
            "item.completed" => {
                if let Some(item) = event.get("item")
                    && item.get("type").and_then(serde_json::Value::as_str) == Some("agent_message")
                    && let Some(text) = item.get("text").and_then(serde_json::Value::as_str)
                    && let Some(p) = progress.as_mut()
                {
                    p.record_text(text.chars().count() as u64);
                }
            }
            "turn.failed" | "error" => {
                // A failure payload can carry the doomed turn's usage — take it when present, so the
                // errored run's record shows what the failure actually consumed.
                if let Some(u) = event.get("usage") {
                    usage_raw = Some(u.clone());
                }
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
        },
    )?;
    // Parse whatever usage the stream reported before the run ended, however it ended — a malformed
    // object surfaces as a parse error rather than being swallowed to zeros (the honest ground-truth
    // contract, cf. parse_result's loud bails).
    let usage: Option<CodexUsage> = usage_raw
        .map(serde_json::from_value)
        .transpose()
        .context("codex's `usage` object was malformed")?;
    // A run that failed after spending still gets its cost recorded: carry the failure as a value with
    // the captured usage (the claude contract — [`Synthesis::error`]), so the caller logs an *errored*
    // run instead of losing the spend on a bail. No captured usage → an all-zeros record: the honest
    // shape for a run that died before any turn completed.
    let error = if let Some(msg) = failure {
        Some(format!("codex reported an error: {msg}"))
    } else if !status.success() {
        Some(format!("codex exited with {}: {}", status, stderr.trim()))
    } else {
        None
    };
    if let Some(error) = error {
        let usage = usage.unwrap_or_default();
        return Ok(Synthesis {
            text: String::new(),
            usage: Usage {
                model: req.model.to_owned(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens + usage.reasoning_output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: usage.cached_input_tokens,
                cost_usd: None,
            },
            structured: None,
            error: Some(error),
        });
    }
    let usage = usage.context("codex produced no `turn.completed` usage event")?;
    // The result is codex's `-o` artifact (clean + schema-valid) — its documented result channel, so
    // an absent/empty one on an otherwise-successful run is surfaced rather than papered over, the way
    // the claude path bails when its result event is missing.
    let text = crate::read_optional(&out_path)?
        .filter(|s| !s.trim().is_empty())
        .context("codex exited successfully but wrote no result to its `-o` artifact")?;
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
        error: None, // a completed run; failures returned above, carrying their captured usage
    })
}

// AI-output handling (parse_result) and the prompt estimate are exercised by
// using `summarize` — its cost/usage output makes any breakage immediately
// apparent — rather than via unit tests.
