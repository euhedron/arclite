use crate::cli::{GlobalArgs, SynthArgs};
use crate::commands::inspect;
use crate::synth::{self, SynthOptions};

/// Build the synthesis prompt from deterministic repo facts.
fn build_prompt(facts_json: &str) -> String {
    format!(
        "You are assessing a code repository from deterministic facts gathered by a tool.\n\
         Facts (JSON):\n{facts_json}\n\n\
         In 3-5 sentences, give a concise, useful assessment of this repository: what kind \
         of project it appears to be, its apparent stack, and anything notable or worth a \
         closer look. Base it only on the facts provided."
    )
}

/// Synthesize a brief assessment of a repository (the `summarize` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let report = inspect::gather(&args.path)?;
    let facts = serde_json::to_string_pretty(&report)?;
    let prompt = build_prompt(&facts);

    synth::run(
        &prompt,
        &SynthOptions {
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
