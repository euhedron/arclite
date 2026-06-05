pub mod audit;
pub mod doctor;
pub mod extract;
pub mod inspect;
pub mod suggest;
pub mod summarize;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// Shared flow for the AI synthesis commands (`summarize`/`suggest`/`extract`/`audit`): gather the
/// repo context once, let the command build its prompt around it, then run — so the commands can't
/// drift in how they wire context, tools, the granted dir, or cost reporting.
pub fn run_synthesis(
    args: &SynthArgs,
    global: &GlobalArgs,
    command: &str,
    build_prompt: impl FnOnce(&str) -> String,
) -> anyhow::Result<()> {
    let ctx = synth::gather_context(
        &args.path,
        &args.include,
        args.rules.as_deref(),
        args.max_file_chars,
        args.changed,
    )?;
    let prompt = build_prompt(&ctx.text);
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
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
