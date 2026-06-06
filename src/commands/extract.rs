use crate::cli::{GlobalArgs, SynthArgs};

/// Extract reusable rules (standards, anti-patterns, principles) from a repository (the `extract` command).
///
/// `suggest` finds repo-specific issues; `extract` abstracts the recurring ones — and the
/// standards a repo enforces — into discrete, repo-agnostic rules. Output is *candidate* rules
/// for a human to curate into a rules dir; they shape every future run, so quality matters.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    super::run_synthesis(args, global, "extract", None, |ctx| {
        format!(
            "You are extracting reusable engineering rules from a code repository — coding \
             standards, anti-patterns, principles, and best-practices that generalize beyond this \
             one repo.\n\n\
             {ctx}\n\
             From the context above, propose a small set of discrete, reusable rules. Favor \
             anti-patterns and violated principles actually evidenced in the code over generic \
             advice, and ground each in something concrete you can point to. Output each rule as:\n\n\
             ## <short-kebab-case-id>\n\
             <one tight paragraph stating the principle/anti-pattern and how to recognize it>\n\
             _provenance: <where in this repo it came from>_\n\n\
             Keep each body to a single paragraph (rules are included verbatim into future runs). \
             Skip anything you can't ground in the context above; treat any rules already present \
             as existing policy and don't duplicate them."
        )
    })
}
