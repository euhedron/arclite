use crate::cli::{GlobalArgs, SynthArgs};
use crate::commands::inspect;
use crate::synth::{self, SynthOptions};

/// Synthesize a brief assessment of a repository (the `summarize` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let report = inspect::gather(&args.path)?;

    // It's all just context — assemble whatever is available, tracking each source.
    let mut ctx = format!(
        "Repository scan (JSON):\n{}\n",
        serde_json::to_string_pretty(&report)?
    );
    let mut sources = vec!["repository scan".to_owned()];

    ctx.push_str(&synth::gather_includes(&args.include, &mut sources));
    ctx.push_str(&synth::gather_rules(args.rules.as_deref(), &mut sources)?);

    // Surface the by-default exclusion so it isn't hidden (and isn't the model's job to flag).
    let excluded = if args.include.is_empty() {
        vec!["the repo's source files (--include <path> to add)".to_owned()]
    } else {
        Vec::new()
    };

    let prompt = format!(
        "You are assessing a code repository from the context below.\n\n\
         {ctx}\n\
         In 3-5 sentences, give a concise, useful assessment: what kind of project it appears \
         to be, its apparent stack, and anything notable or worth a closer look. Respect any \
         rules above; ground it in the context."
    );

    synth::run(
        &prompt,
        &SynthOptions {
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            sources: &sources,
            excluded: &excluded,
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
