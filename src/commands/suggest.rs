use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `suggest` structured-output item: one suggestion with its rationale.
const SUGGEST_ITEM: &str = r#"{"type":"object","properties":{"suggestion":{"type":"string"},"rationale":{"type":"string"}},"required":["suggestion","rationale"]}"#;

/// Suggest's attention taxonomy (the (label, description) dual use: see [`Structure`]'s `kinds`).
const SUGGEST_KINDS: &[(&str, &str)] = &[
    ("risk", "something fragile or hazardous worth hardening"),
    ("improvement", "working code or docs that could be clearer or simpler"),
    ("unfinished", "something started but not yet complete"),
    ("verification", "an assumption or claim worth confirming"),
    ("awareness", "context worth knowing, with no action implied"),
];

/// The `suggest` command.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(SUGGEST_ITEM),
        note: "each item: `suggestion`, `rationale`.",
        kinds: SUGGEST_KINDS,
    };
    super::run_synthesis(args, global, "suggest", Some(structure), |ctx| {
        format!(
            "You are reviewing a code repository to advise where attention is best spent. Consider \
             these kinds of suggestion:\n{}\n\n\
             {ctx}\n\
             Produce concrete suggestions, each a short line with a one-clause rationale.",
            super::kind_list(SUGGEST_KINDS)
        )
    })
}
