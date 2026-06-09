use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `evolve` structured-output item: one radical proposal.
const EVOLVE_ITEM: &str = r#"{"type":"object","properties":{"change":{"type":"string"},"rationale":{"type":"string"}},"required":["change","rationale"]}"#;

/// The `evolve` command.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(EVOLVE_ITEM),
        note: "\n\nReturn the result as structured data — each item with `change` (a radical, drastic direction) and `rationale` (why it could be worth it despite seeming extreme).",
    };
    super::run_synthesis(args, global, "evolve", Some(structure), |ctx| {
        format!(
            "You are exploring how this repository could radically evolve.\n\n\
             {ctx}\n\
             Propose drastic overhauls, structural reimaginings, and bold directions that would \
             normally go unspoken — challenge the fundamental assumptions, scope, and shape of the \
             project. What would a fresh attempt, unburdened by the current design, do differently? \
             Treat what exists as a point of departure, not a constraint. Ground each proposal in \
             something concrete above; for each, give the change and why it could be worth it despite \
             seeming extreme."
        )
    })
}
