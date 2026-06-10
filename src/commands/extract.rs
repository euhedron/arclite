use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `extract` structured-output item: one proposed rule.
const EXTRACT_ITEM: &str = r#"{"type":"object","properties":{"id":{"type":"string"},"rule":{"type":"string"},"provenance":{"type":"string"}},"required":["id","rule","provenance"]}"#;

/// The `extract` command. Output is *candidate* rules for a human to curate into a rules dir.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(EXTRACT_ITEM),
        note: "\n\nReturn the result as structured data — one object per proposed rule, each with `id` (short kebab-case), `rule` (one tight paragraph stating the principle/anti-pattern and how to recognize it), and `provenance` (where in this repo it came from). Empty if there are none worth proposing.",
    };
    super::run_synthesis(args, global, "extract", Some(structure), |ctx| {
        format!(
            "You are extracting reusable engineering rules from a code repository — coding \
             standards, anti-patterns, principles, and best-practices that generalize beyond this \
             one repo.\n\n\
             {ctx}\n\
             From the context above, propose a small set of discrete, reusable rules — each with a \
             short kebab-case id, one tight paragraph stating the principle/anti-pattern and how to \
             recognize it, and its provenance (where in this repo it came from). Favor anti-patterns \
             and violated principles actually evidenced in the code over generic advice, and ground \
             each in something concrete. Keep each to a single paragraph (rules are included verbatim \
             into future runs). Skip anything you can't ground in the context above; treat any rules \
             already present as existing policy and don't duplicate them."
        )
    })
}
