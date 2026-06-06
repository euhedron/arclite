pub mod audit;
pub mod critique;
pub mod doctor;
pub mod extract;
pub mod inspect;
pub mod suggest;
pub mod summarize;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// An optional structured-output mode a command can offer: a JSON Schema the model's result is
/// validated against (returned as `structured_output`), plus a prompt note describing the shape.
/// Used only when `--structured` is passed; commands without one reject the flag. The structure is
/// command-appropriate — audit's violations ≠ suggest's ranked list — never one schema for all.
pub struct Structure {
    pub schema: &'static str,
    pub note: &'static str,
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
) -> anyhow::Result<()> {
    let ctx = synth::gather_context(
        &args.path,
        &args.include,
        args.rules.as_deref(),
        args.max_file_chars,
        args.changed,
    )?;
    let mut prompt = build_prompt(&ctx.text);
    // --structured: emit the command's typed output (schema-validated), if it defines one.
    let schema = if args.structured {
        let s = structure.as_ref().ok_or_else(|| {
            anyhow::anyhow!("`{command}` has no structured output mode — drop --structured")
        })?;
        prompt.push_str(s.note);
        Some(s.schema)
    } else {
        None
    };
    synth::run(
        &prompt,
        &SynthOptions {
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            dir: &ctx.root,
            sources: &ctx.sources,
            excluded: &ctx.excluded,
            command,
            output: args.output.as_deref(),
            ambient_memory: args.ambient_memory,
            schema,
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
