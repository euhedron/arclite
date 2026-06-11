use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `critique` structured-output item: one defect and where it is.
const CRITIQUE_ITEM: &str = r#"{"type":"object","properties":{"location":{"type":"string"},"defect":{"type":"string"}},"required":["location","defect"]}"#;

/// Critique's defect taxonomy: the descriptions are the categories it surfaces (listed in its
/// prompt), the labels its `--kinds` vocabulary — one declaration feeding both.
const CRITIQUE_KINDS: &[(&str, &str)] = &[
    ("redundancy", "the same thing stated or built in more than one place"),
    ("inconsistency", "parts that contradict each other"),
    ("staleness", "claims that no longer match reality"),
    ("gap", "missing pieces or unhandled cases"),
    ("dead", "unused or unreachable elements"),
    ("tightening", "what could be consolidated, restructured, or clarified"),
];

/// The `critique` command.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(CRITIQUE_ITEM),
        note: "each item: `location`, `defect`.",
        kinds: CRITIQUE_KINDS,
    };
    super::run_synthesis(args, global, "critique", Some(structure), |ctx| {
        format!(
            "You are performing a rigorous critical review of a repository and its documentation to \
             surface quality defects of these kinds:\n{}\n\n\
             {ctx}\n\
             Report concrete findings; for each, the specific location and the problem in a clause. \
             Prefer fewer real findings over padding, and call out cross-cutting redundancy explicitly.",
            super::kind_list(CRITIQUE_KINDS)
        )
    })
}
