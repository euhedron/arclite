use std::process::ExitCode;

use crate::cli::{GlobalArgs, SynthArgs};

/// Critically review a repository and its docs for quality defects worth fixing (the `critique` command).
///
/// Distinct from its siblings by *lens*, not subject: `suggest` prioritizes what's worth attention,
/// `audit` flags violations of provided rules, `summarize` describes — `critique` hunts imperfection:
/// redundancy, inconsistency, staleness, gaps, dead weight, and consolidation/clarity opportunities,
/// each with a concrete fix. Reach for it to harden a codebase or its docs against their own sloppiness.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
    super::run_synthesis(args, global, "critique", None, |ctx| {
        format!(
            "You are performing a rigorous critical review of a repository and its documentation to \
             surface quality defects worth fixing — not a priority list, and not rule violations, but \
             imperfections: redundancy and duplication, inconsistencies and contradictions, staleness \
             (claims that no longer match reality), gaps and omissions, dead or unused elements, and \
             opportunities to consolidate, restructure, clarify, or tighten.\n\n\
             {ctx}\n\
             Report concrete findings grouped by kind. For each: the specific location, the problem in a \
             clause, and the concrete fix you would make. Ground every finding in the context above — flag \
             nothing you can't point to. Prefer fewer real findings over padding, and call out cross-cutting \
             redundancy (the same thing stated or implemented in several places) explicitly."
        )
    })
}
