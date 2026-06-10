use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `audit` structured-output item: one concrete rule violation.
const AUDIT_ITEM: &str = r#"{"type":"object","properties":{"rule":{"type":"string"},"location":{"type":"string"},"reason":{"type":"string"}},"required":["rule","location","reason"]}"#;

/// The `audit` command.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    let structure = Structure {
        schema: crate::synth::results_schema(AUDIT_ITEM),
        note: "one object per concrete violation, each with `rule` (the rule id), `location` (file/area), and `reason` (one clause). Empty if there are none.",
    };
    super::run_synthesis(args, global, "audit", Some(structure), |ctx| {
        format!(
            "You are auditing a code repository strictly against the rules provided below (listed \
             under \"Rules\").\n\n\
             {ctx}\n\
             Report only concrete violations of those rules. For each: the rule id, the file/location \
             where it occurs, and a one-clause reason it violates. Do not raise general suggestions, \
             and do not mention rules that aren't violated. If no rules are present in the context, \
             say exactly that and stop."
        )
    })
}
