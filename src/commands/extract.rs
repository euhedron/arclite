use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `extract` structured-output item: one proposed rule.
const EXTRACT_ITEM: &str = r#"{"type":"object","properties":{"id":{"type":"string"},"rule":{"type":"string"},"provenance":{"type":"string"}},"required":["id","rule","provenance"]}"#;

/// The `extract` command. Output is *candidate* rules for a human to curate into a rules dir.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(EXTRACT_ITEM),
        note: "one object per proposed rule: `id`, `rule`, `provenance`.",
        kinds: &[], // no fixed taxonomy; --kinds lets the model label freely
    };
    super::run_synthesis(args, global, "extract", Some(structure), |ctx| {
        format!(
            "You are extracting reusable engineering rules from a code repository — coding \
             standards, anti-patterns, principles, and best-practices that generalize beyond this \
             one repo.\n\n\
             {ctx}\n\
             From the context above, propose any discrete, reusable rules the repo clearly evidences \
             — each with a short kebab-case id, one tight paragraph stating the principle/anti-pattern \
             and how to recognize it, and its provenance (where in this repo it came from). Favor \
             anti-patterns and violated principles actually evidenced in the code over generic \
             advice. Keep each to a single paragraph (rules are included verbatim into future runs). \
             Treat any rules already present as existing policy and don't duplicate them. Propose \
             only rules that clearly earn their place by a general principle that holds across repos \
             — never pad the set or manufacture generic advice to reach a count. If nothing beyond \
             existing policy is clearly warranted, return no rules and say so in the note: an empty, \
             honestly-explained result is a valid and useful outcome, not a failure."
        )
    })
}
