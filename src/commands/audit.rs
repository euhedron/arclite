use crate::cli::{GlobalArgs, SynthArgs};

/// Audit a repository against the provided rules, flagging only violations (the `audit` command).
///
/// Where `suggest` gives an open-ended review, `audit` is narrow: enforce exactly the rules in
/// context (`--rules <dir>`) and report only where the code breaks them.
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    super::run_synthesis(args, global, "audit", |ctx| {
        format!(
            "You are auditing a code repository strictly against the rules provided below (listed \
             under \"Rules\").\n\n\
             {ctx}\n\
             Report only concrete violations of those rules. For each: the rule id, the file/location \
             where it occurs, and a one-clause reason it violates. Do not raise general suggestions, \
             and do not mention rules that aren't violated. Ground every finding in the context above \
             — flag nothing you can't point to. If no rules are present in the context, say exactly \
             that and stop."
        )
    })
}
