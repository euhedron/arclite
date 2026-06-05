use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// Synthesize a prioritized list of suggestions for a repository (the `suggest` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let ctx = synth::gather_context(
        &args.path,
        &args.include,
        args.rules.as_deref(),
        args.max_file_chars,
    )?;

    let prompt = format!(
        "You are reviewing a code repository to advise where attention is best spent.\n\n\
         {}\n\
         Produce a prioritized list (most important first) of concrete suggestions — what to \
         look at, verify, improve, finish, or be aware of — each a short line with a one-clause \
         rationale. Treat any rules above as the policy to check against. Ground every item in \
         the context above; skip anything you can't support.",
        ctx.text
    );

    synth::run(
        &prompt,
        &SynthOptions {
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            dir: &ctx.root,
            sources: &ctx.sources,
            excluded: &ctx.excluded,
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
