use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// How [`Usage::model`] was established — response-derived ground truth, or the requested id echoed
/// back because the backend's events name no model. Serialized into every run record and shown in the
/// run report, so a substitution-blind backend's records never *read* as confirmed identity
/// (report-the-identity-that-ran).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// The response's own per-model usage named it (claude's `modelUsage`).
    Reported,
    /// The backend echoes no model id, so this is the *requested* one, unconfirmed (codex; or a
    /// claude error payload that named no model).
    Requested,
}

/// Token usage and cost for one synthesis call — ground truth from the CLI's response. `cost_usd` is
/// `Some` when the CLI returns an authoritative dollar cost (claude), and `None` when the backend
/// reports token usage but no cost (codex reports tokens only — no fabricated estimate).
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    /// The display form of `models` — its members joined with " + " when several ran. Rendering
    /// only; code that needs the identities reads `models`, never re-splits this prose.
    pub model: String,
    /// The identity set as data: every model the response confirmed ran (or the one requested id
    /// when nothing was confirmed). The structured source `model` is derived from; fan-out
    /// aggregation merges these, not the display strings. Serialized only when it says more than
    /// `model` does (several members).
    #[serde(skip_serializing_if = "one_or_fewer")]
    pub models: Vec<String>,
    /// Whether `model` came from the response or is the unconfirmed requested id — disclosed in the
    /// report and the record, never silently presented as the identity that ran.
    pub model_source: ModelSource,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cost_usd: Option<f64>,
    /// True when `cost_usd` is a *lower bound*: a fan-out summed members where some reported a
    /// dollar cost and some couldn't (an errored member with unknown spend). Displayed as "≥" and
    /// recorded, so a partial total is never presented as exact. `false` for every single run.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cost_partial: bool,
    /// True when the run's spend is *unknown* — the child ran but returned no usage, so the zeros
    /// here are placeholders, not measurements. Recorded so `arc usage` counts these runs as
    /// unknown-spend rather than folding them into the ordinary token sums as if zero were real.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub spend_unknown: bool,
}

/// `skip_serializing_if` for [`Usage::models`]: a single (or absent) identity adds nothing over
/// the `model` field it derives.
#[allow(clippy::ptr_arg)] // serde's skip_serializing_if passes the field type by reference
fn one_or_fewer(models: &Vec<String>) -> bool {
    models.len() <= 1
}

/// A synthesis result: the model's text plus what it cost.
#[derive(Debug, Clone, Serialize)]
pub struct Synthesis {
    pub text: String,
    pub usage: Usage,
    /// Schema-validated structured output — present whenever the verb declares a shape (every verb
    /// but summarize). Read this for the typed result instead of parsing `text`.
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
    /// Names the failure on an error payload (e.g. `error_max_budget_usd`). Not always meaningful:
    /// a quota-limit refusal arrives as `is_error` with subtype `"success"` and the real message in
    /// `result` (confirmed by exercise), so error detail must not read `subtype` alone.
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

/// The one display join for a model identity set — [`Usage::model`]'s documented derivation
/// (members joined with " + "). Shared with fan-out aggregation ([`crate::synth`]'s `sum_usage`),
/// so the separator has a single home.
pub(crate) fn join_models(models: &[String]) -> String {
    models.join(" + ")
}

/// The four token counters summed across a payload's `modelUsage` entries, as one tuple
/// (input, output, cache-creation, cache-read) — the single summation both of `parse_result`'s
/// payload paths read (the error payload's real spend, the incomplete payload's salvage), so how
/// modelUsage totals are computed can't drift between them.
fn model_usage_totals(
    model_usage: &std::collections::BTreeMap<String, PerModelUsage>,
) -> (u64, u64, u64, u64) {
    (
        model_usage.values().map(|m| m.input_tokens).sum(),
        model_usage.values().map(|m| m.output_tokens).sum(),
        model_usage
            .values()
            .map(|m| m.cache_creation_input_tokens)
            .sum(),
        model_usage
            .values()
            .map(|m| m.cache_read_input_tokens)
            .sum(),
    )
}

/// The model identity a payload supports: a non-empty confirmed set (the response's own per-model
/// usage) becomes the identity that ran, [`ModelSource::Reported`]; an empty one falls back to the
/// requested id, disclosed as [`ModelSource::Requested`] — never presented as confirmed. The single
/// resolution shared by every parse path (error, incomplete, success), so the fallback semantics
/// can't drift between branches.
fn model_identity(confirmed: &[String], requested: &str) -> (String, Vec<String>, ModelSource) {
    if confirmed.is_empty() {
        (
            requested.to_owned(),
            vec![requested.to_owned()],
            ModelSource::Requested,
        )
    } else {
        (
            join_models(confirmed),
            confirmed.to_vec(),
            ModelSource::Reported,
        )
    }
}

/// Parse the Claude CLI JSON payload into a [`Synthesis`]. The model reported is resolved from the
/// payload's per-model usage — the models that actually ran — never echoed from the request, so a
/// substitution can't mislabel the run.
pub fn parse_result(json: &str, requested_model: &str) -> anyhow::Result<Synthesis> {
    let parsed: ClaudeJson =
        serde_json::from_str(json).context("claude did not return the expected JSON")?;
    // The identity that ran, as the payload actually confirms it: one modelUsage entry names the
    // model outright; several confirm only the *set* that ran (the CLI's auxiliary models bill
    // small calls), so the set is kept as data (`models`) and reported joined for display — never
    // one member presented as though it alone produced the synthesis, and never a display string
    // downstream code would have to re-split. Resolved once, shared by the success and error paths.
    let confirmed: Vec<String> = parsed.model_usage.keys().cloned().collect();
    if parsed.is_error.unwrap_or(false) {
        // A run that ran and *spent* but did not complete (e.g. a tripped --max-budget-usd cap). On an
        // error payload the top-level `usage` block is zeroed (confirmed by exercise) while the real
        // tokens are in `modelUsage` and the real cost in `total_cost_usd` — so the honest usage sums
        // modelUsage rather than reading the zeros, and the failure is carried as a value (logged), not
        // bailed (which would lose the spend).
        let (model, models, model_source) = model_identity(&confirmed, requested_model);
        let (input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens) =
            model_usage_totals(&parsed.model_usage);
        let usage = Usage {
            model,
            models,
            model_source,
            input_tokens,
            output_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            cost_usd: parsed.total_cost_usd,
            cost_partial: false,
            // With no modelUsage entries at all, the zeros are placeholders (nothing was measured);
            // with entries, the sums are the payload's own numbers.
            spend_unknown: parsed.model_usage.is_empty(),
        };
        // Prefer human-readable detail — the `errors` list, then a `result` message (a quota-limit
        // refusal carries its message there, under subtype "success") — before the bare `subtype`
        // code, then a placeholder. An errored run's `arc log` entry shows this string; a run that
        // errored as "success" would read as a contradiction and hide the actual cause.
        let detail = if !parsed.errors.is_empty() {
            parsed.errors.join("; ")
        } else if let Some(msg) = parsed.result.filter(|r| !r.trim().is_empty()) {
            msg
        } else {
            parsed
                .subtype
                .unwrap_or_else(|| "no detail in the payload".to_owned())
        };
        return Ok(Synthesis {
            text: String::new(),
            usage,
            structured: None,
            error: Some(detail),
        });
    }
    // usage, cost, and per-model identification are part of a successful response's contract. A
    // payload missing any of them is *semantically incomplete* — surfaced loudly as an errored run,
    // but carrying every field that DID parse (the modelUsage token sums, the cost if present),
    // never discarding real parsed spend for all-zero placeholders
    // (account-for-consumed-cost-on-failure).
    let mut missing = Vec::new();
    if parsed.result.is_none() {
        missing.push("`result`");
    }
    if parsed.usage.is_none() {
        missing.push("`usage`");
    }
    if parsed.total_cost_usd.is_none() {
        missing.push("`total_cost_usd`");
    }
    if confirmed.is_empty() {
        missing.push("`modelUsage`");
    }
    // Whatever token counts DID parse, kept ahead of the completeness check: the top-level `usage`
    // block when present, else the modelUsage sums — so an incomplete payload salvages every parsed
    // field rather than discarding one source because the other is missing.
    let salvaged_tokens = parsed.usage.as_ref().map_or_else(
        || model_usage_totals(&parsed.model_usage),
        |u| {
            (
                u.input_tokens,
                u.output_tokens,
                u.cache_creation_input_tokens,
                u.cache_read_input_tokens,
            )
        },
    );
    let usage_absent = parsed.usage.is_none();
    // Resolved once for whichever path follows: confirmed set → Reported, none → the requested id
    // disclosed as Requested.
    let (model, models, model_source) = model_identity(&confirmed, requested_model);
    let complete = (
        parsed.result,
        parsed.usage,
        parsed.total_cost_usd,
        (!confirmed.is_empty()).then_some(()),
    );
    let (Some(text), Some(usage), Some(cost_usd), Some(())) = complete else {
        let (input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens) =
            salvaged_tokens;
        return Ok(Synthesis {
            text: String::new(),
            usage: Usage {
                model,
                models,
                model_source,
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                cost_usd: parsed.total_cost_usd,
                cost_partial: false,
                // The authoritative counters went missing, so whatever was salvaged is a *lower
                // bound*, not a measurement: marked unknown whenever the top-level `usage` block is
                // absent (modelUsage sums are a fallback view) — and, of course, when nothing
                // parsed at all. The rollup then counts this run apart instead of misreading it as
                // an ordinary token-only (codex-style) run.
                spend_unknown: usage_absent,
            },
            structured: None,
            error: Some(format!(
                "claude's success payload was missing {} — recorded with the usage that did parse",
                missing.join(", ")
            )),
        });
    };
    Ok(Synthesis {
        text,
        usage: Usage {
            model,
            models,
            // Reported by construction: this path requires a non-empty confirmed set above.
            model_source,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cost_usd: Some(cost_usd),
            cost_partial: false,
            spend_unknown: false,
        },
        structured: parsed.structured_output,
        error: None,
    })
}

/// The [`Usage`] for a run whose child *ran* but returned no usage at all: zeros marked
/// `spend_unknown` — placeholders, never measurements — under the necessarily-requested model id.
/// The one shape every no-usage failure path records, so none of them can register as a measured
/// zero-token run.
fn unknown_spend_usage(requested_model: &str) -> Usage {
    Usage {
        model: requested_model.to_owned(),
        models: vec![requested_model.to_owned()],
        model_source: ModelSource::Requested,
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        cost_usd: None,
        cost_partial: false,
        spend_unknown: true,
    }
}

/// An errored synthesis for a run whose child *ran* but whose spend is unknowable — the payload was
/// unparseable, or the usage event never arrived. Carried as a value so the run still reaches the
/// log as errored with its cause, rather than vanishing on a bail after tokens may have burned
/// (account-for-consumed-cost-on-failure).
fn errored_without_usage(requested_model: &str, message: String) -> Synthesis {
    Synthesis {
        text: String::new(),
        usage: unknown_spend_usage(requested_model),
        structured: None,
        error: Some(message),
    }
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

/// Each backend's name, named once: the [`BACKENDS`] registry rows, [`DEFAULT_BACKEND`], and every
/// cross-module reference (the config table's provider-listing rows) derive from these, so a rename
/// is one edit here — never a scattered literal hunt.
pub(crate) const CLAUDE: &str = "claude";
pub(crate) const CODEX: &str = "codex";

/// arclite's default synthesis backend, used when neither `--backend` nor `defaults.backend` is set.
pub const DEFAULT_BACKEND: &str = CLAUDE;

/// Constructs a backend instance — the factory half of a [`BACKENDS`] registry row.
type BackendFactory = fn() -> Box<dyn Backend>;

/// The known synthesis backends: each name paired with its `Backend` constructor and a one-line
/// capability blurb — the one registry `backend()` dispatches from, `known_backends()` lists, and
/// the `--backend` CLI help enumerates, so adding a backend is a single row here, not a name list,
/// a `match` arm, and a help string kept in lockstep. `doctor` probes, `validate_backend`, and
/// error wording all derive from the name set.
const BACKENDS: &[(&str, BackendFactory, &str)] = &[
    (
        CLAUDE,
        || Box::new(ClaudeBackend),
        "reports dollar cost; honors --max-budget-usd and --allow-tool",
    ),
    (
        CODEX,
        || Box::new(CodexBackend),
        "reports tokens only — no dollar cost, no native budget cap, no tool grants",
    ),
];

/// The known backend names, derived from the [`BACKENDS`] registry.
pub(crate) fn known_backends() -> Vec<&'static str> {
    BACKENDS.iter().map(|(name, _, _)| *name).collect()
}

/// The `--backend` flag's help text, derived from the [`BACKENDS`] registry rows (names, default
/// marker, capability blurbs) — so the CLI's enumeration can't go stale against the registry.
pub(crate) fn backends_help() -> String {
    let list = BACKENDS
        .iter()
        .map(|(name, _, blurb)| {
            let default = if *name == DEFAULT_BACKEND {
                " (default)"
            } else {
                ""
            };
            format!("`{name}`{default}: {blurb}")
        })
        .collect::<Vec<_>>()
        .join(". ");
    format!("Synthesis backend — {list}. Overrides the configured `defaults.backend`")
}

/// The providers' model-listing endpoints and Anthropic's pinned API version (the value the API
/// docs' own example pins) — named here as each URL's one home.
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models?limit=1000";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

/// Anthropic's model listing (claude's provider): `x-api-key` + the pinned `anthropic-version`,
/// newest first per the API's contract; `limit=1000` (the documented maximum) makes truncation
/// practically impossible, and `has_more` is still surfaced rather than assumed false.
fn anthropic_models(settings: &Settings) -> anyhow::Result<ModelListing> {
    let (key, key_source) = anthropic_key(settings)?
        .ok_or_else(|| key_hint(ANTHROPIC_KEY_ENV, ANTHROPIC_KEY_SETTING))?;
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        display_name: Option<String>,
    }
    #[derive(Deserialize)]
    struct Page {
        data: Vec<Entry>,
        #[serde(default)]
        has_more: bool,
    }
    // Redirects off, fail closed: no redirect is part of this API's contract, and `x-api-key` is a
    // custom header curl would forward to whatever host a redirect named — no stripping exists for it.
    let body = crate::http::get(
        ANTHROPIC_MODELS_URL,
        &[("anthropic-version", ANTHROPIC_API_VERSION)],
        &[("x-api-key", &key)],
        false,
        None,
    )
    .context("fetching Anthropic's model list")?;
    let page: Page = serde_json::from_str(&body).context("parsing Anthropic's model list")?;
    Ok(ModelListing {
        models: page
            .data
            .into_iter()
            .map(|e| ModelEntry {
                id: e.id,
                display_name: e.display_name,
            })
            .collect(),
        key_source,
        truncated: page.has_more,
        // Anthropic's listing is ordered by the provider, not sorted here by timestamp — no
        // undated-entry caveat exists to disclose.
        undated: 0,
    })
}

/// OpenAI's model listing (codex's provider): `Authorization: Bearer`, the whole set in one page.
/// The API documents no order, so entries sort newest first by `created`; the ids are the API org's
/// models — for a ChatGPT-subscription codex login that account's lineup may differ, and the listing's
/// key provenance is disclosed so the report says whose account it reflects.
fn openai_models(settings: &Settings) -> anyhow::Result<ModelListing> {
    let (key, key_source) =
        openai_key(settings)?.ok_or_else(|| key_hint(OPENAI_KEY_ENV, OPENAI_KEY_SETTING))?;
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        // Optional, honestly: an absent timestamp must not become a fabricated epoch-zero that
        // silently sorts the entry as "oldest" — undated entries sort after the dated ones and the
        // count is disclosed on the listing.
        created: Option<i64>,
    }
    #[derive(Deserialize)]
    struct Page {
        data: Vec<Entry>,
    }
    let bearer = format!("Bearer {key}");
    // Redirects off, fail closed: no redirect is part of this API's contract either — refused rather
    // than followed, matching the Anthropic listing's posture.
    let body = crate::http::get(
        OPENAI_MODELS_URL,
        &[],
        &[("Authorization", &bearer)],
        false,
        None,
    )
    .context("fetching OpenAI's model list")?;
    let mut page: Page = serde_json::from_str(&body).context("parsing OpenAI's model list")?;
    // Undated entries sort after the dated ones (never fabricated into epoch zero), and the count
    // rides the listing itself — TUI worker threads call this while the TUI owns the terminal, so
    // no printing here; each surface shows the disclosure its own way.
    let undated = page.data.iter().filter(|e| e.created.is_none()).count();
    page.data
        .sort_by_key(|e| std::cmp::Reverse(e.created.map_or((0, 0), |c| (1, c))));
    Ok(ModelListing {
        models: page
            .data
            .into_iter()
            .map(|e| ModelEntry {
                id: e.id,
                display_name: None,
            })
            .collect(),
        key_source,
        // Not a silent default: OpenAI's `/v1/models` is unpaginated by contract — the whole set
        // arrives in one response, and no `has_more`-style signal exists to read (unlike Anthropic's,
        // which the sibling surfaces). `false` states that contract.
        truncated: false,
        undated,
    })
}

/// The claude backend's default model. Update when a newer model supersedes it; the run reports the
/// resolved id the response returns.
const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// The codex backend's default model — specified explicitly (not read from codex's own `config.toml`)
/// so a codex run is self-contained. Update as codex's lineup advances; the run reports the id used.
/// Verified callable through arc's own codex path before becoming the default — the repo's evidence
/// is run `1784084700-52822-826236000` in the run log (verify-a-model-is-callable-not-just-listed).
const DEFAULT_CODEX_MODEL: &str = "gpt-5.6-sol";

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

    /// The provider's live model listing for this backend — `Ok` newest-first, or an error naming
    /// what's missing (no API key) or what failed. The authoritative enumeration: neither agent CLI
    /// lists models headlessly, but each provider's `/v1/models` does, keyed by the provider's
    /// standard env var or a saved user-layer `api_keys.*` setting.
    fn list_models(&self, settings: &Settings) -> anyhow::Result<ModelListing>;

    /// Where this backend's provider API key would come from (`Ok(Some(source))`), `Ok(None)` when
    /// no key is available, or the resolution's own failure (a set-but-non-unicode env var) —
    /// resolved exactly as [`Backend::list_models`] resolves it, so the status doctor reports and
    /// the listing's behavior can't disagree.
    fn model_key_source(&self, settings: &Settings) -> anyhow::Result<Option<String>>;

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
    /// error, never silently dropped. Default: honor everything, but no backend accepts a tool name
    /// shaped like an option: the names ride a variadic CLI flag, so a leading-dash value would
    /// escape its argument slot into the agent CLI's own grammar
    /// (guard-values-interpolated-into-commands).
    fn reject_unsupported(
        &self,
        max_budget_usd: Option<f64>,
        allowed_tools: &[String],
    ) -> anyhow::Result<()> {
        let _ = max_budget_usd;
        for tool in allowed_tools {
            anyhow::ensure!(
                !tool.starts_with('-') && !tool.is_empty(),
                "--allow-tool value `{tool}` looks like an option, not a tool name — it would escape its argument slot"
            );
        }
        Ok(())
    }

    /// The reasoning effort this backend runs at, given any configured value — surfaced in the report
    /// and applied to the call, because it shapes cost. Default: `None` (the backend has no such knob).
    /// A backend with one returns the effective value (the configured one, else its own default).
    fn reasoning_effort(&self, configured: Option<&str>) -> Option<String> {
        let _ = configured;
        None
    }

    /// Whether this backend reports an authoritative dollar cost on its runs. Default: it does. A
    /// tokens-only backend (codex) overrides to `false`, so consumers can tell a record that lacks
    /// cost *by design* from one that lost a cost it should have carried.
    fn reports_cost(&self) -> bool {
        true
    }
}

/// Select a synthesis backend by name — dispatched from the single [`BACKENDS`] registry.
pub fn backend(name: &str) -> anyhow::Result<Box<dyn Backend>> {
    BACKENDS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, make, _)| make())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown backend `{name}` (known: {})",
                known_backends().join(", ")
            )
        })
}

/// One provider-reported model: its id, and a human display name where the provider gives one.
pub struct ModelEntry {
    pub id: String,
    pub display_name: Option<String>,
}

/// A provider's model listing plus its provenance: where the key came from (disclosed, so the report
/// says whose account the list reflects), whether pagination truncated it, and how many entries
/// carried no `created` timestamp (sorted last, not as oldest) — every caveat carried as data for
/// each surface to show, never printed from here (a TUI worker may be calling).
pub struct ModelListing {
    pub models: Vec<ModelEntry>,
    pub key_source: String,
    pub truncated: bool,
    pub undated: usize,
}

/// Each provider's key env var and saved-setting key — one home per name, shared by the listing
/// fetch, the doctor status, and the no-key hint, so the three can't drift.
const ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const ANTHROPIC_KEY_SETTING: &str = "api_keys.anthropic";
const OPENAI_KEY_ENV: &str = "OPENAI_API_KEY";
const OPENAI_KEY_SETTING: &str = "api_keys.openai";

/// Resolve a provider API key: the standard env var wins (a session override), else the saved
/// user-layer setting. `Some((key, source))` discloses where it came from; `None` = no key — the
/// same resolution backs the listings and doctor's status line, so they can't disagree. A set-but-
/// non-unicode env value is an error, not the absent case: silently sliding past a mangled
/// credential to the saved key (or to "no key") would swap whose account gets used without a word.
fn provider_key(
    env_var: &str,
    saved: Option<&str>,
    setting_key: &str,
) -> anyhow::Result<Option<(String, String)>> {
    match std::env::var(env_var) {
        Ok(key) if !key.is_empty() => {
            return Ok(Some((key, format!("{env_var} (environment)"))));
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => {}
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!(
            "{env_var} is set but not valid unicode — fix or unset it (refusing to silently fall back to the saved key)"
        ),
    }
    Ok(saved.map(|key| (key.to_owned(), format!("{setting_key} (user settings)"))))
}

/// The no-key error for a model listing — names both ways to supply one. The save path reads the
/// key from stdin, so the recommendation never puts a secret on argv or into shell history.
fn key_hint(env_var: &str, setting_key: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "no API key for the model listing — set {env_var}, or save one with `arc config set {setting_key}` (reads the key from stdin; auto-saved to the user layer)"
    )
}

/// Anthropic's key (claude's provider), resolved per [`provider_key`].
fn anthropic_key(settings: &Settings) -> anyhow::Result<Option<(String, String)>> {
    provider_key(
        ANTHROPIC_KEY_ENV,
        settings.api_key_anthropic.as_deref(),
        ANTHROPIC_KEY_SETTING,
    )
}

/// OpenAI's key (codex's provider), resolved per [`provider_key`].
fn openai_key(settings: &Settings) -> anyhow::Result<Option<(String, String)>> {
    provider_key(
        OPENAI_KEY_ENV,
        settings.api_key_openai.as_deref(),
        OPENAI_KEY_SETTING,
    )
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

    fn list_models(&self, settings: &Settings) -> anyhow::Result<ModelListing> {
        anthropic_models(settings)
    }

    fn model_key_source(&self, settings: &Settings) -> anyhow::Result<Option<String>> {
        Ok(anthropic_key(settings)?.map(|(_, source)| source))
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
///
/// The two failure classes stay distinguishable for the accounting contract: a *launch* failure
/// (spawn) is a hard `Err` — nothing ran, nothing spent — while any error after the child is live
/// (a broken stream read, a failed wait) returns through [`DriveError::AfterSpawn`], because the
/// child may have metered tokens by then and the caller must record an errored run, never bail
/// (account-for-consumed-cost-on-failure).
enum DriveError {
    /// The process never started; nothing spent. Carries the launch error.
    Launch(anyhow::Error),
    /// The process ran, then the drive failed; spend is possible and must be accounted.
    AfterSpawn(anyhow::Error),
}

/// What [`drive`] hands back for a child that ran to exit: its status, captured stderr, and the
/// prompt write's failure if any — carried as data (not an error) because its meaning depends on
/// the outcome: under a child-reported failure it's corroborating detail, under a claimed success
/// it means the result came from a truncated prompt and the caller must not report it unqualified.
struct Driven {
    status: std::process::ExitStatus,
    stderr: String,
    prompt_write_error: Option<std::io::Error>,
}

fn drive(
    mut cmd: Command,
    prompt: &str,
    launch_err: &'static str,
    mut on_event: impl FnMut(&str, &serde_json::Value, &str),
) -> Result<Driven, DriveError> {
    cmd.current_dir(std::env::temp_dir()) // neutral cwd; the agent's working root is set per-backend
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .context(launch_err)
        .map_err(DriveError::Launch)?;
    // Write the prompt on its own thread, concurrently with draining stdout/stderr below: writing it
    // all first (a large prompt on a small pipe buffer, notably Windows) deadlocks if the child emits
    // output before consuming stdin — its output pipe fills while we're blocked writing stdin.
    // The write's outcome IS captured (joined below): the prompt is the run's essential input, not
    // best-effort bookkeeping. EVERY failure is reported — including a broken pipe: the child chose
    // to stop reading (often a budget rejection whose own error explains itself), but if it then
    // claims *success*, that success was computed against a truncated prompt and must not stand
    // unqualified. Dropping stdin when the closure ends signals end-of-input.
    let mut stdin = child.stdin.take().expect("stdin was configured as piped");
    let prompt = prompt.to_owned();
    let stdin_writer = std::thread::spawn(move || stdin.write_all(prompt.as_bytes()).err());
    // Drain stderr on its own thread, concurrently with the stdout stream below: a backend that fills
    // the stderr pipe buffer while we're still reading stdout would block writing it — and we'd never
    // reach a post-loop read — a deadlock. Joined after the child exits. A failed drain is *marked* in
    // the captured text rather than silently truncating it — downstream error messages quote this
    // stderr, and a partial capture must not read as the whole story.
    let mut stderr_pipe = child.stderr.take().expect("stderr was configured as piped");
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr = String::new();
        if let Err(e) = stderr_pipe.read_to_string(&mut stderr) {
            stderr.push_str(&format!(" [stderr capture failed partway: {e}]"));
        }
        stderr
    });
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    for line in std::io::BufReader::new(stdout).lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                // The stream is broken but the child is still running — and still metering. Stop it
                // and reap it before reporting; a kill/reap that *itself* fails is named in the
                // error (the child may still be consuming), never silently discarded.
                let mut cleanup = String::new();
                if let Err(kill) = child.kill() {
                    cleanup.push_str(&format!(" (killing the agent failed: {kill} — it may still be running and consuming)"));
                }
                if let Err(reap) = child.wait() {
                    cleanup.push_str(&format!(" (reaping the agent failed: {reap})"));
                }
                return Err(DriveError::AfterSpawn(anyhow::Error::new(e).context(
                    format!("reading the agent CLI's output stream{cleanup}"),
                )));
            }
        };
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue; // non-JSON noise (e.g. a stdin warning) — skip
        };
        let kind = event
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        on_event(kind, &event, &line);
    }
    let status = child.wait().map_err(|e| {
        DriveError::AfterSpawn(anyhow::Error::new(e).context("waiting for the agent CLI"))
    })?;
    let stderr = stderr_reader
        .join()
        .expect("the stderr reader thread panicked");
    let prompt_write_error = stdin_writer
        .join()
        .expect("the stdin writer thread panicked");
    Ok(Driven {
        status,
        stderr,
        prompt_write_error,
    })
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
    // Structured output (a verb-declared shape): the result returns as a schema-validated
    // `structured_output` object, never scraped from prose.
    if let Some(schema) = req.json_schema {
        cmd.arg("--json-schema").arg(schema);
    }
    // Isolation covers the agent's whole ambient configuration, and `--ambient-memory` re-enables
    // the whole of it — memory AND settings — which the flag's help says in as many words, so
    // nothing rides back in undisclosed. Memory: user/project CLAUDE.md + auto-memory (a neutral cwd
    // alone does NOT stop the user-level one). Settings: `--setting-sources ""`, or hooks and other
    // behavior-shaping config would load under a run arclite states explicitly. Auth is a separate
    // store, unaffected either way (confirmed by exercise).
    if !req.ambient_memory {
        cmd.env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1");
        cmd.env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");
        cmd.args(["--setting-sources", ""]);
    }
    let mut result_line: Option<String> = None;
    let driven = drive(
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
    );
    // Launch failures bail (nothing ran, nothing spent); any failure after the child was live —
    // a broken stream read, a failed wait — is a metered run whose spend is unknown, carried as an
    // errored synthesis so it reaches the log (account-for-consumed-cost-on-failure).
    let Driven {
        status,
        stderr,
        prompt_write_error,
    } = match driven {
        Ok(driven) => driven,
        Err(DriveError::Launch(e)) => return Err(e),
        Err(DriveError::AfterSpawn(e)) => {
            // The stream died — but if the `result` event already arrived, its usage is real,
            // parsed spend: record THAT (as an errored run naming the stream failure), falling to
            // unknown-zeros only when no payload ever landed.
            if let Some(line) = &result_line
                && let Ok(mut salvaged) = parse_result(line, req.model)
            {
                salvaged.error = Some(format!(
                    "claude's stream failed after its result arrived ({e:#}) — usage salvaged from the captured payload{}",
                    salvaged
                        .error
                        .map(|prior| format!("; the payload itself reported: {prior}"))
                        .unwrap_or_default()
                ));
                salvaged.text = String::new();
                salvaged.structured = None;
                return Ok(salvaged);
            }
            return Ok(errored_without_usage(
                req.model,
                format!("claude's stream failed after launch — spend unknown: {e:#}"),
            ));
        }
    };
    // A failed run usually still emits a `result` error event (e.g. a tripped --max-budget-usd cap:
    // is_error + subtype) — parse that for the real failure rather than reporting a bare exit code.
    // The child *ran* on every path below, so tokens may have burned even where no payload came back:
    // each failure is carried as an errored synthesis (spend unknown → zeros, disclosed by the error
    // text) rather than bailed, which would drop the run from the log and its cost from accounting.
    let result_line = match (result_line, status.success()) {
        (Some(line), _) => line,
        (None, false) => {
            return Ok(errored_without_usage(
                req.model,
                format!(
                    "claude exited with {} and no `result` event — spend unknown: {}",
                    status,
                    stderr.trim()
                ),
            ));
        }
        (None, true) => {
            return Ok(errored_without_usage(
                req.model,
                "claude exited successfully but produced no `result` event — spend unknown"
                    .to_owned(),
            ));
        }
    };
    let mut synthesis = match parse_result(&result_line, req.model) {
        Ok(synthesis) => synthesis,
        Err(e) => {
            return Ok(errored_without_usage(
                req.model,
                format!("claude's result payload didn't parse — spend unknown: {e:#}"),
            ));
        }
    };
    // A non-zero exit whose payload parsed as an error is the expected failed-run shape — the failure
    // is carried in `synthesis.error` (with its real usage) for logging. A non-zero exit that parsed
    // as a *success* is a genuine contradiction — but its usage is real, billed spend, so it too is
    // carried as an errored run (loudly naming the contradiction) rather than bailed and lost.
    if !status.success() && synthesis.error.is_none() {
        synthesis.error = Some(format!(
            "claude exited with {status} despite a success result"
        ));
    }
    // A claimed success whose prompt never fully arrived isn't one: the synthesis ran against a
    // truncated input, so it's carried as errored (its real usage intact). Under an already-failed
    // run the child's own error stands — the write failure is its side effect, not the story.
    if let Some(e) = prompt_write_error
        && synthesis.error.is_none()
    {
        synthesis.error = Some(format!(
            "claude reported success but stopped reading the prompt partway ({e}) — the result reflects a truncated prompt"
        ));
    }
    Ok(synthesis)
}

/// The codex backend's *default* reasoning effort, used when `defaults.codex_reasoning_effort` isn't
/// set — specified explicitly (not read from codex's `config.toml`) so a run is self-contained, and
/// surfaced in the run report since it shapes cost. The highest tier, matching the audit/critique role
/// where judgment quality matters more than latency.
const CODEX_REASONING_EFFORT: &str = "xhigh";

/// The reasoning-effort levels codex's `model_reasoning_effort` accepts (per its config reference;
/// update as the lineup changes). [`CODEX_REASONING_EFFORT`] (the default) must be one of these.
/// Shared with the config key's option list, so the picker and the validator can't drift.
pub(crate) const CODEX_REASONING_EFFORTS: &[&str] =
    &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Validate a configured / `config set` backend name against the known set — delegating to [`backend`],
/// the single authority — so a typo is rejected at set + load time, not only when a run tries to use it.
pub(crate) fn validate_backend(name: &str) -> anyhow::Result<()> {
    backend(name).map(|_| ())
}

/// Validate a model id wherever one enters (an explicit `--model`, a configured default): it is
/// emitted as the value after the child CLI's `--model` option, so an option-shaped id (leading
/// dash) would escape its value slot into the CLI's argument grammar, and an empty one would leave
/// the option dangling (guard-values-interpolated-into-commands).
pub(crate) fn validate_model_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !id.is_empty() && !id.starts_with('-'),
        "`{id}` doesn't look like a model id — it would escape its `--model` value slot"
    );
    Ok(())
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

    fn list_models(&self, settings: &Settings) -> anyhow::Result<ModelListing> {
        openai_models(settings)
    }

    fn model_key_source(&self, settings: &Settings) -> anyhow::Result<Option<String>> {
        Ok(openai_key(settings)?.map(|(_, source)| source))
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

    /// codex reports token usage but no dollar cost — its records are tokens-only by design.
    fn reports_cost(&self) -> bool {
        false
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

impl CodexUsage {
    /// The one CodexUsage → [`Usage`] mapping — the errored and success returns both report through
    /// it. Codex doesn't echo a per-model id in its events, so the model is the *requested* one,
    /// marked [`ModelSource::Requested`] — disclosed in the report and the record, never presented
    /// as response-confirmed identity (unlike claude, which resolves it from per-model usage).
    fn into_usage(self, model: &str) -> Usage {
        Usage {
            model: model.to_owned(),
            models: vec![model.to_owned()],
            model_source: ModelSource::Requested,
            input_tokens: self.input_tokens,
            // Codex separates reasoning tokens; fold them into output for an honest total-generated
            // count.
            output_tokens: self.output_tokens + self.reasoning_output_tokens,
            cache_creation_input_tokens: 0, // codex has no cache-creation concept, only cached reads
            cache_read_input_tokens: self.cached_input_tokens,
            cost_usd: None, // codex reports tokens only — no fabricated dollar cost
            cost_partial: false,
            spend_unknown: false,
        }
    }
}

/// A per-run temp directory for codex's file-based `--output-schema`/`-o`, removed on drop. Unique
/// per call so concurrent `--runs N` codex runs can't collide on the schema/output files.
struct CodexTemp(PathBuf);

impl CodexTemp {
    fn new() -> anyhow::Result<Self> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("arclite-codex-{}-{n}", std::process::id()));
            // create_dir, not create_dir_all: an existing directory — a leftover from a dead process
            // whose pid was reused, since Drop's cleanup is best-effort — must not be adopted, or its
            // stale out.txt could be read back as this run's result. On collision, take the next name;
            // success guarantees the directory is freshly created and empty.
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("cannot create codex temp dir {}", dir.display())
                    });
                }
            }
        }
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
    let driven = drive(
        cmd,
        req.prompt,
        "failed to launch `codex` — is the Codex CLI installed and on PATH?",
        |kind, event, _raw| match kind {
            "turn.completed" => {
                // Capture the raw usage object; the strict parse happens after the stream (below),
                // where a malformed object can bail loudly.
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
    );
    // Parse whatever usage the stream reported before the run ended, however it ended. A malformed
    // usage object is surfaced loudly — but as an *errored run* (the child ran; its spend is real
    // even though unreadable), never a bail that would drop the run from the log entirely.
    let usage: Option<CodexUsage> = match usage_raw.map(serde_json::from_value).transpose() {
        Ok(usage) => usage,
        Err(e) => {
            return Ok(errored_without_usage(
                req.model,
                format!("codex's `usage` object was malformed — spend unknown: {e}"),
            ));
        }
    };
    // Launch failures bail (nothing ran, nothing spent). Any failure after the child was live — a
    // broken stream read, a failed wait — is a metered run: with captured usage it's recorded as
    // that spend; without, as spend-unknown zeros. Either way an errored run reaches the log
    // (account-for-consumed-cost-on-failure), never a bail that loses it.
    let Driven {
        status,
        stderr,
        prompt_write_error,
    } = match driven {
        Ok(driven) => driven,
        Err(DriveError::Launch(e)) => return Err(e),
        Err(DriveError::AfterSpawn(e)) => {
            if let Some(usage) = usage {
                return Ok(Synthesis {
                    text: String::new(),
                    usage: usage.into_usage(req.model),
                    structured: None,
                    error: Some(format!("codex's stream failed after spend: {e:#}")),
                });
            }
            return Ok(errored_without_usage(
                req.model,
                format!("codex's stream failed after launch — spend unknown: {e:#}"),
            ));
        }
    };
    // A run that failed after spending still gets its cost recorded: carry the failure as a value with
    // the captured usage (the claude contract — [`Synthesis::error`]), so the caller logs an *errored*
    // run instead of losing the spend on a bail. No captured usage → spend UNKNOWN, marked as such —
    // never a default-zeros record that would register as a measured zero-token run.
    let error = if let Some(msg) = failure {
        Some(format!("codex reported an error: {msg}"))
    } else if !status.success() {
        Some(format!("codex exited with {}: {}", status, stderr.trim()))
    } else {
        None
    };
    if let Some(error) = error {
        return Ok(Synthesis {
            text: String::new(),
            usage: usage.map_or_else(
                || unknown_spend_usage(req.model),
                |u| u.into_usage(req.model),
            ),
            structured: None,
            error: Some(error),
        });
    }
    let usage = match usage {
        Some(usage) => usage.into_usage(req.model),
        None => {
            // A success exit with no usage event: the run demonstrably ran, its spend is unknown —
            // an errored record (zeros, disclosed) rather than a bail that loses the run.
            return Ok(errored_without_usage(
                req.model,
                "codex exited successfully but reported no `turn.completed` usage event — spend unknown"
                    .to_owned(),
            ));
        }
    };
    // From here the run has demonstrably spent (its usage is in hand), so a failed *result read* — a
    // missing/empty `-o` artifact, an unreadable one, or structured output that isn't the schema'd
    // JSON — is carried as a value with that usage, never bailed: the errored-run contract, so the
    // spend reaches the log even when the result is unusable.
    let errored = |message: String| Synthesis {
        text: String::new(),
        usage: usage.clone(),
        structured: None,
        error: Some(message),
    };
    // The result is codex's `-o` artifact (clean + schema-valid) — its documented result channel, so
    // an absent/empty one on an otherwise-successful run is surfaced rather than papered over.
    let text = match crate::read_optional(&out_path) {
        Ok(Some(text)) if !text.trim().is_empty() => text,
        Ok(_) => {
            return Ok(errored(
                "codex exited successfully but wrote no result to its `-o` artifact".to_owned(),
            ));
        }
        Err(e) => {
            return Ok(errored(format!(
                "codex's `-o` artifact could not be read: {e}"
            )));
        }
    };
    let structured = if req.json_schema.is_some() {
        match serde_json::from_str(text.trim()) {
            Ok(value) => Some(value),
            Err(e) => {
                return Ok(errored(format!(
                    "codex did not return the expected JSON for the requested schema: {e}"
                )));
            }
        }
    } else {
        None
    };
    // A claimed success whose prompt never fully arrived isn't one — same contract as the claude
    // path: the result reflects a truncated input, carried as errored with its real usage.
    if let Some(e) = prompt_write_error {
        return Ok(Synthesis {
            text: String::new(),
            usage,
            structured: None,
            error: Some(format!(
                "codex completed but stopped reading the prompt partway ({e}) — the result reflects a truncated prompt"
            )),
        });
    }
    Ok(Synthesis {
        text,
        usage,
        structured,
        error: None, // a completed run; failures returned above, carrying their captured usage
    })
}

// AI-output handling (parse_result) and the prompt estimate are exercised by
// using `summarize` — its cost/usage output makes any breakage immediately
// apparent — rather than via unit tests.
