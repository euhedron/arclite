use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `audit` structured-output mode: a `violations` array (empty = clean) — findings as data, not
/// prose. `gate: "violations"` makes it the collection `--fail-on-findings` blocks on (non-empty ⇒
/// non-zero exit); no verdict/pass-fail convention, just the findings themselves.
const AUDIT_STRUCTURE: Structure = Structure {
    schema: r#"{"type":"object","properties":{"violations":{"type":"array","items":{"type":"object","properties":{"rule":{"type":"string"},"location":{"type":"string"},"reason":{"type":"string"}},"required":["rule","location","reason"]}}},"required":["violations"]}"#,
    note: "\n\nReturn the result as structured data: a `violations` array — one object per concrete violation, each with `rule` (the rule id), `location` (file/area), and `reason` (one clause). Empty array if there are none.",
    gate: Some("violations"),
};

/// Audit a repository against the provided rules, flagging only violations (the `audit` command):
/// enforce exactly the rules in context and report only where the code breaks them.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    super::run_synthesis(args, global, "audit", Some(AUDIT_STRUCTURE), |ctx| {
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
