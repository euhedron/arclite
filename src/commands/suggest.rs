use crate::cli::{GlobalArgs, SynthArgs};

/// Synthesize a prioritized list of suggestions for a repository (the `suggest` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    super::run_synthesis(args, global, "suggest", |ctx| {
        format!(
            "You are reviewing a code repository to advise where attention is best spent.\n\n\
             {ctx}\n\
             Produce a prioritized list (most important first) of concrete suggestions — what to \
             look at, verify, improve, finish, or be aware of — each a short line with a one-clause \
             rationale. Treat any rules above as the policy to check against. Ground every item in \
             the context above; skip anything you can't support."
        )
    })
}
