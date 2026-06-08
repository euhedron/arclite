pub mod audit;
pub mod critique;
pub mod doctor;
pub mod extract;
pub mod inspect;
pub mod suggest;
pub mod summarize;

use std::process::ExitCode;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// An optional structured-output mode a command can offer: a JSON Schema the model's result is
/// validated against (returned as `structured_output`), plus a prompt note describing the shape.
/// Used only when `--structured` is passed; commands without one reject the flag. The structure is
/// command-appropriate — audit's violations ≠ suggest's ranked list — never one schema for all.
///
/// `gate` names the array field whose non-emptiness means "block" (audit → `violations`), making
/// gate-ability a property the command *declares* rather than a special case bolted onto one verb:
/// `--fail-on-findings` turns that field into a non-zero exit. `None` = the structure carries no
/// problem semantics (e.g. `suggest`'s ranked list), so the flag is rejected for it, not ignored.
pub struct Structure {
    pub schema: &'static str,
    pub note: &'static str,
    pub gate: Option<&'static str>,
}

/// Grounding guardrail appended to every synthesis prompt (single-sourced, not restated per prompt).
const GROUNDING: &str =
    "\n\nGround everything you report in the context above; include nothing you cannot point to in it.";

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
    let settings = crate::settings::Settings::load(&args.path)?;
    let rule_sources = resolve_rule_sources(args, &settings)?;
    let model = args
        .model
        .clone()
        .or_else(|| settings.default_model.clone());
    // Per-run logging is on by default; a user/project setting (`defaults.logging = false`) disables it.
    let log = settings.default_logging != Some(false);
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
        &rule_sources,
        args.max_file_chars,
        args.changed,
    )?;
    let mut prompt = build_prompt(&ctx.text);
    prompt.push_str(GROUNDING);
    // --structured emits the command's typed output; --fail-on-findings additionally gates on it
    // (and implies it). Both require the command to define a structure; gating also requires that
    // structure to declare a `gate` field — so the flag is rejected, not silently ignored, otherwise.
    let want_structured = args.structured || args.fail_on_findings;
    let (schema, gate) = if want_structured {
        let s = structure.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "`{command}` has no structured output mode — drop --structured/--fail-on-findings"
            )
        })?;
        prompt.push_str(s.note);
        let gate = if args.fail_on_findings {
            Some(s.gate.ok_or_else(|| {
                anyhow::anyhow!(
                    "`{command}` has no findings to gate on — its structured output isn't a violations/findings collection (try `audit`)"
                )
            })?)
        } else {
            None
        };
        (Some(s.schema), gate)
    } else {
        (None, None)
    };
    synth::run(
        &prompt,
        &SynthOptions {
            model: model.as_deref(),
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

/// Resolve which rule sources to load, in precedence order: an ad-hoc `--rules <path>`, else a
/// named `--ruleset <id>` (or the configured `defaults.ruleset`) from settings, else none.
fn resolve_rule_sources(
    args: &SynthArgs,
    settings: &crate::settings::Settings,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    if let Some(path) = &args.rules {
        return Ok(vec![path.clone()]);
    }
    let Some(id) = args
        .ruleset
        .as_deref()
        .or(settings.default_ruleset.as_deref())
    else {
        return Ok(Vec::new());
    };
    settings
        .ruleset(id)
        .map(<[std::path::PathBuf]>::to_vec)
        .ok_or_else(|| anyhow::anyhow!("ruleset `{id}` is not defined in .arc/settings.json"))
}
