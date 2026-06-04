use crate::cli::{GlobalArgs, SynthArgs};
use crate::commands::inspect;
use crate::synth::{self, SynthOptions, read_capped};

/// Synthesize a prioritized list of suggestions for a repository (the `suggest` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let report = inspect::gather(&args.path)?;
    let root = std::path::absolute(&args.path).unwrap_or_else(|_| args.path.clone());

    // It's all just context — assemble whatever is available, tracking each source.
    let mut ctx = format!(
        "Repository scan (JSON):\n{}\n",
        serde_json::to_string_pretty(&report)?
    );
    let mut sources = vec!["repository scan".to_owned()];

    if let Some(readme) = read_capped(&root.join("README.md"), 4000) {
        sources.push(format!("README.md ({} chars)", readme.chars().count()));
        ctx.push_str(&format!("\nREADME:\n{readme}\n"));
    }
    for name in &report.manifests {
        if let Some(body) = read_capped(&root.join(name), 2000) {
            sources.push(format!("{name} ({} chars)", body.chars().count()));
            ctx.push_str(&format!("\n{name}:\n{body}\n"));
        }
    }
    ctx.push_str(&synth::gather_includes(&args.include, &mut sources));
    ctx.push_str(&synth::gather_rules(args.rules.as_deref(), &mut sources)?);

    // Surface the by-default exclusion so it isn't hidden (and isn't the model's job to flag).
    let excluded = if args.include.is_empty() {
        vec!["the repo's source files (--include <path> to add)".to_owned()]
    } else {
        Vec::new()
    };

    let prompt = format!(
        "You are reviewing a code repository to advise where attention is best spent.\n\n\
         {ctx}\n\
         Produce a prioritized list (most important first) of concrete suggestions — what to \
         look at, verify, improve, finish, or be aware of — each a short line with a one-clause \
         rationale. Treat any rules above as the policy to check against. Ground every item in \
         the context above; skip anything you can't support."
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
