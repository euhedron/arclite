use std::process::ExitCode;

use crate::cli::{GlobalArgs, SynthArgs};

/// Critically review a repository for quality defects (the `critique` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    super::run_synthesis(args, global, "critique", None, |ctx| {
        format!(
            "You are performing a rigorous critical review of a repository and its documentation to \
             surface quality defects: redundancy and duplication, inconsistencies and contradictions, \
             staleness (claims that no longer match reality), gaps and omissions, dead or unused \
             elements, and opportunities to consolidate, restructure, clarify, or tighten.\n\n\
             {ctx}\n\
             Report concrete findings grouped by kind. For each: the specific location and the problem \
             in a clause. Prefer fewer real findings over padding, and call out cross-cutting \
             redundancy (the same thing stated or implemented in several places) explicitly."
        )
    })
}
