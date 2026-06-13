pub mod audit;
pub mod config;
pub mod critique;
pub mod doctor;
pub mod evolve;
pub mod extract;
pub mod init;
pub mod inspect;
pub mod log;
pub mod rules;
pub mod status;
pub mod suggest;
pub mod summarize;
pub mod usage;

use std::process::ExitCode;

use anyhow::Context;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// An optional structured-output mode a command can offer: a JSON Schema the model's result is
/// validated against (returned as `structured_output`), plus a note describing its item shape
/// (appended to the shared [`STRUCTURED_NOTE`] framing).
/// The schema is the shared `results`-array envelope ([`crate::synth::results_schema`]) wrapping the
/// command's own item shape — so commands declare only what differs. Used only when `--structured`
/// is passed; commands without one reject the flag. The gate, `--ranked`, `--kinds`, and multi-run
/// aggregation all treat the `results` array uniformly; `--fail-on-findings` blocks when it's
/// non-empty.
pub struct Structure {
    pub schema: String,
    pub note: &'static str,
    /// The command's kind taxonomy as (label, description) pairs, declared once. The command lists
    /// it in its own prompt (via [`kind_list`]) as the substance of what it looks for; `--kinds`
    /// reuses the labels as the suggested classification vocabulary. Empty = no taxonomy (`--kinds`
    /// then lets the model label freely). "taxonomy" not "criteria" — one general description per
    /// label, not a checklist of conditions.
    pub kinds: &'static [(&'static str, &'static str)],
}

/// Grounding guardrail appended to every synthesis prompt (single-sourced, not restated per prompt).
const GROUNDING: &str = "\n\nGround everything you report in the context above; include nothing you cannot point to in it.";

/// Appended by `--ranked`: order the results by significance (the array order is the ranking).
const RANKED_NOTE: &str =
    "\n\nOrder the results from most to least significant; the order is the ranking.";

/// Shared framing for structured output, prepended to the command's own item-shape note
/// (single-sourced like [`GROUNDING`]/[`RANKED_NOTE`], so it can't drift between commands).
const STRUCTURED_NOTE: &str = "\n\nReturn the result as structured data — ";

/// Appended after the command's item-shape note: every structured run also returns a required
/// top-level `note`, so an empty `results` is a judged outcome rather than silence.
const NOTE_INSTRUCTION: &str = " Also include a top-level `note`: one or two clauses giving the overall read of the run (what was assessed, and the upshot) — especially when `results` is empty.";

/// Render a command's kind taxonomy ([`Structure`]'s `kinds`) as a labelled list — `- label:
/// description` per line — for the command to weave into its own prompt. (Why one declaration serves
/// both the prompt and `--kinds`: see that field.)
pub(crate) fn kind_list(kinds: &[(&str, &str)]) -> String {
    kinds
        .iter()
        .map(|(label, description)| format!("- {label}: {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Appended by `--kinds`: ask for a per-item `kind`. With a declared taxonomy (already listed in the
/// command's own prompt) the model picks from it but may use its own label when none fit — a
/// deviation that is itself signal about the taxonomy's fit; with none, it labels freely. Like
/// `--ranked`, this shapes the output in any mode; the classification is the lever's, never a
/// command's prompt.
fn kinds_note(has_taxonomy: bool) -> &'static str {
    if has_taxonomy {
        "\n\nAlso give each result a `kind` — one of the kinds listed above, or your own if none fit."
    } else {
        "\n\nAlso give each result a `kind` — its category of finding."
    }
}

/// Shared flow for the AI synthesis commands: gather the repo context once, let the command build
/// its prompt around it, then run — so the commands can't drift in how they wire context, tools,
/// the granted dir, cost reporting, or structured output. `structure` is the command's optional
/// structured-output mode (see [`Structure`]); `--structured` activates it, or errors if absent.
pub fn run_synthesis(
    args: &SynthArgs,
    global: &GlobalArgs,
    command: &str,
    structure: Option<Structure>,
    build_prompt: impl FnOnce(&str) -> String,
) -> anyhow::Result<ExitCode> {
    anyhow::ensure!(
        (1..=crate::synth::MAX_RUNS).contains(&args.runs),
        "--runs must be between 1 and {}, got {}",
        crate::synth::MAX_RUNS,
        args.runs
    );
    let settings = crate::settings::Settings::load(&args.path)?;
    let resolution =
        resolve_rule_sources(args.rules.as_deref(), args.ruleset.as_deref(), &settings)?;
    // Backend: the `--backend` flag over the configured default, else arclite's default. The resolved
    // instance owns the per-backend policy below — which model default applies, whether a native spend
    // cap is honored, and which requested capabilities it can't — so this function never branches on
    // the backend name (that lives only in `ai::backend`, the single home of the known backends).
    let backend_name = args
        .backend
        .clone()
        .or_else(|| settings.default_backend.clone())
        .unwrap_or_else(|| crate::ai::DEFAULT_BACKEND.to_owned());
    let backend = crate::ai::backend(&backend_name)?;
    // Reject, before any spend, a requested capability this backend can't honor — surfaced as an
    // error, never silently dropped.
    backend.reject_unsupported(args.max_budget_usd, &args.allow_tool)?;
    let model = backend.resolve_model(args.model.as_deref(), backend.configured_model(&settings));
    let max_budget_usd =
        backend.resolve_budget(args.max_budget_usd, settings.default_max_budget_usd);
    let reasoning_effort =
        backend.reasoning_effort(settings.default_codex_reasoning_effort.as_deref());
    let log = settings.logging_enabled();
    // Disclose which settings layers are active (user then project) in the run output — configuration
    // detected and in effect is reported, never left for the reader to infer.
    let config = settings.active_display();
    let ctx = synth::gather_context(
        &args.path,
        &args.include,
        &resolution.sources,
        args.max_file_chars,
        args.changed,
    )?;
    let mut prompt = build_prompt(&ctx.text);
    prompt.push_str(GROUNDING);
    // --structured emits the command's typed output; --fail-on-findings additionally gates on it.
    // Both require the command to define a structure, so the flag is rejected — not silently
    // ignored — when a command has none.
    let want_structured = args.structured || args.fail_on_findings;
    let (schema, gate) = if want_structured {
        let s = structure.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "`{command}` has no structured output mode — drop --structured/--fail-on-findings"
            )
        })?;
        prompt.push_str(STRUCTURED_NOTE);
        prompt.push_str(s.note);
        prompt.push_str(NOTE_INSTRUCTION);
        // Gate on the `results` array the schemas produce — the key single-sourced in synth.
        let gate = args.fail_on_findings.then_some(crate::synth::RESULTS_KEY);
        // --kinds adds a free-string `kind` to each item — not enum-locked, so the model can label
        // off the command's suggested taxonomy when none fits (the deviation is signal).
        let schema = if args.kinds {
            synth::with_kind(&s.schema)?
        } else {
            s.schema.clone()
        };
        (Some(schema), gate)
    } else {
        (None, None)
    };
    // --kinds and --ranked shape the output in any mode (a prompt note; structured runs also carry
    // it in the `kind` field / array order above) — neither requires structured output.
    if args.kinds {
        let has_taxonomy = structure.as_ref().is_some_and(|s| !s.kinds.is_empty());
        prompt.push_str(kinds_note(has_taxonomy));
    }
    if args.ranked {
        prompt.push_str(RANKED_NOTE);
    }
    synth::run(
        &prompt,
        &SynthOptions {
            model: &model,
            backend: &backend_name,
            runs: args.runs,
            max_budget_usd,
            reasoning_effort: reasoning_effort.as_deref(),
            ranked: args.ranked,
            kinds: args.kinds,
            allowed_tools: &args.allow_tool,
            dir: &ctx.root,
            sources: &ctx.sources,
            excluded: &ctx.excluded,
            config: &config,
            command,
            output: args.output.as_deref(),
            ambient_memory: args.ambient_memory,
            schema: schema.as_deref(),
            gate,
            dry_run: args.dry_run,
            json: global.json,
            log,
        },
    )
}

/// What `--rules`/`--ruleset`/`defaults.ruleset` resolved to: a human description of the selection
/// (for reporting) plus the source paths to load. Shared by `run_synthesis` and `arc rules`.
pub(crate) struct RuleResolution {
    pub description: String,
    pub sources: Vec<std::path::PathBuf>,
}

/// Resolve which rule sources to load, in precedence order: an ad-hoc `--rules <path>`, else a
/// named `--ruleset <id>` (or the configured `defaults.ruleset`) from settings, else none.
pub(crate) fn resolve_rule_sources(
    rules: Option<&std::path::Path>,
    ruleset: Option<&str>,
    settings: &crate::settings::Settings,
) -> anyhow::Result<RuleResolution> {
    if let Some(path) = rules {
        return Ok(RuleResolution {
            description: format!("ad-hoc rules: {}", path.display()),
            sources: vec![path.to_path_buf()],
        });
    }
    let from_flag = ruleset.is_some();
    let Some(id) = ruleset.or(settings.default_ruleset.as_deref()) else {
        return Ok(RuleResolution {
            description: "no ruleset selected".to_owned(),
            sources: Vec::new(),
        });
    };
    let sources = settings
        .ruleset(id)
        .map(<[std::path::PathBuf]>::to_vec)
        .ok_or_else(|| anyhow::anyhow!("ruleset `{id}` is not defined in .arc/settings.json"))?;
    Ok(RuleResolution {
        description: format!(
            "ruleset `{id}` (from {})",
            if from_flag {
                "--ruleset"
            } else {
                "defaults.ruleset"
            }
        ),
        sources,
    })
}

/// Resolve `path` to an absolute path with a uniform error — shared by the command entry points, so
/// the resolution and its wording are single-sourced rather than copy-pasted.
pub(crate) fn resolve_root(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))
}
