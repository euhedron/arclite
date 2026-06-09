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

use std::process::ExitCode;

use anyhow::Context;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// An optional structured-output mode a command can offer: a JSON Schema the model's result is
/// validated against (returned as `structured_output`), plus a prompt note describing the shape.
/// The schema is the shared `results`-array envelope ([`crate::synth::results_schema`]) wrapping the
/// command's own item shape — so commands declare only what differs. Used only when `--structured`
/// is passed; commands without one reject the flag. The gate, `--ranked`, and multi-run aggregation
/// all treat the `results` array uniformly; `--fail-on-findings` blocks when it's non-empty.
pub struct Structure {
    pub schema: String,
    pub note: &'static str,
}

/// Grounding guardrail appended to every synthesis prompt (single-sourced, not restated per prompt).
const GROUNDING: &str =
    "\n\nGround everything you report in the context above; include nothing you cannot point to in it.";

/// Appended by `--ranked`: order the results by significance (the array order is the ranking).
const RANKED_NOTE: &str =
    "\n\nOrder the results from most to least significant; the order is the ranking.";

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
    anyhow::ensure!(args.runs >= 1, "--runs must be at least 1, got {}", args.runs);
    let settings = crate::settings::Settings::load(&args.path)?;
    let resolution =
        resolve_rule_sources(args.rules.as_deref(), args.ruleset.as_deref(), &settings)?;
    let model = args
        .model
        .clone()
        .or_else(|| settings.default_model.clone());
    let log = settings.logging_enabled();
    // Disclose which settings layers are active (user then project) in the run output — configuration
    // detected and in effect is reported, never left for the reader to infer.
    let config: Vec<String> = settings
        .active
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let ctx = synth::gather_context(
        &args.path,
        &args.include,
        &resolution.sources,
        args.max_file_chars,
        args.changed,
    )?;
    let mut prompt = build_prompt(&ctx.text);
    prompt.push_str(GROUNDING);
    // --structured emits the command's typed output; --fail-on-findings additionally gates on it
    // (and implies it). Both require the command to define a structure — so the flag is rejected,
    // not silently ignored, when a command has none.
    let want_structured = args.structured || args.fail_on_findings;
    let (schema, gate) = if want_structured {
        let s = structure.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "`{command}` has no structured output mode — drop --structured/--fail-on-findings"
            )
        })?;
        prompt.push_str(s.note);
        // Gate on the `results` array the schemas produce — the key single-sourced in synth.
        let gate = args.fail_on_findings.then_some(crate::synth::RESULTS_KEY);
        (Some(s.schema.as_str()), gate)
    } else {
        (None, None)
    };
    if args.ranked {
        prompt.push_str(RANKED_NOTE);
    }
    synth::run(
        &prompt,
        &SynthOptions {
            model: model.as_deref(),
            runs: args.runs,
            allowed_tools: &args.allow_tool,
            dir: &ctx.root,
            sources: &ctx.sources,
            excluded: &ctx.excluded,
            config: &config,
            command,
            output: args.output.as_deref(),
            ambient_memory: args.ambient_memory,
            schema,
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
            if from_flag { "--ruleset" } else { "defaults.ruleset" }
        ),
        sources,
    })
}

/// Resolve `path` to an absolute path with a uniform error — shared by the command entry points, so
/// the resolution and its wording are single-sourced rather than copy-pasted.
pub(crate) fn resolve_root(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))
}
