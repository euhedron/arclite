use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `suggest` structured-output item: one suggestion with its rationale.
const SUGGEST_ITEM: &str = r#"{"type":"object","properties":{"suggestion":{"type":"string"},"rationale":{"type":"string"}},"required":["suggestion","rationale"]}"#;

/// The `suggest` command.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(SUGGEST_ITEM),
        note: "\n\nReturn the result as structured data — each item with `suggestion` (one line) and `rationale` (one clause).",
    };
    super::run_synthesis(args, global, "suggest", Some(structure), |ctx| {
        format!(
            "You are reviewing a code repository to advise where attention is best spent.\n\n\
             {ctx}\n\
             Produce a list of concrete suggestions — what to look at, verify, improve, finish, or \
             be aware of — each a short line with a one-clause rationale. Treat any rules above as \
             the policy to check against."
        )
    })
}
