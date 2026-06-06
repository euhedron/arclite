use crate::cli::{GlobalArgs, SynthArgs};

/// Synthesize a brief assessment of a repository (the `summarize` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    super::run_synthesis(args, global, "summarize", None, |ctx| {
        format!(
            "You are assessing a code repository from the context below.\n\n\
             {ctx}\n\
             In 3-5 sentences, give a concise, useful assessment: what kind of project it appears \
             to be, its apparent stack, and anything notable or worth a closer look. Respect any \
             rules above; ground it in the context."
        )
    })
}
