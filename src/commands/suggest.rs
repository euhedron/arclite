use std::process::ExitCode;

use super::Structure;
use crate::cli::{GlobalArgs, SynthArgs};

/// The `suggest` structured-output mode: a `suggestions` array (ordering is the shared `--ranked`
/// option, not baked in here). A different item shape from audit's findings — same envelope.
const SUGGEST_STRUCTURE: Structure = Structure {
    schema: r#"{"type":"object","properties":{"suggestions":{"type":"array","items":{"type":"object","properties":{"suggestion":{"type":"string"},"rationale":{"type":"string"}},"required":["suggestion","rationale"]}}},"required":["suggestions"]}"#,
    note: "\n\nReturn the result as structured data: a `suggestions` array, each with `suggestion` (one line) and `rationale` (one clause).",
    // `suggest` *can* gate: a non-empty suggestion list signals "room for improvement", which a user
    // may deliberately choose to block on (e.g. to surface refinement opportunities in CI). Gating is
    // a property each command declares, not a privilege reserved for `audit`.
    gate: Some("suggestions"),
};

/// Synthesize a list of suggestions for a repository (the `suggest` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    super::run_synthesis(args, global, "suggest", Some(SUGGEST_STRUCTURE), |ctx| {
        format!(
            "You are reviewing a code repository to advise where attention is best spent.\n\n\
             {ctx}\n\
             Produce a list of concrete suggestions — what to look at, verify, improve, finish, or \
             be aware of — each a short line with a one-clause rationale. Treat any rules above as \
             the policy to check against."
        )
    })
}
