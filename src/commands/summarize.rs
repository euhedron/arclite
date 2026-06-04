use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// Synthesize a brief assessment of a repository (the `summarize` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let ctx = synth::gather_context(&args.path, &args.include, args.rules.as_deref())?;

    let prompt = format!(
        "You are assessing a code repository from the context below.\n\n\
         {}\n\
         In 3-5 sentences, give a concise, useful assessment: what kind of project it appears \
         to be, its apparent stack, and anything notable or worth a closer look. Respect any \
         rules above; ground it in the context.",
        ctx.text
    );

    synth::run(
        &prompt,
        &SynthOptions {
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            sources: &ctx.sources,
            excluded: &ctx.excluded,
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
