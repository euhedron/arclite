//! The AI synthesis verbs, as data. Each verb is the shared synthesis flow ([`run_synthesis`])
//! wrapped around a verb-specific prompt and an optional structured-output shape; collapsing the
//! six near-identical command modules into one table single-sources the build-the-`Structure`-and-
//! run boilerplate, so a change to that shared shape is one edit rather than six.

use std::process::ExitCode;

use super::{Structure, kind_list, run_synthesis};
use crate::cli::{self, GlobalArgs, SynthArgs};

/// One AI verb: its CLI name (single-sourced from [`cli`]'s `NAME_*`), its optional structured-output
/// shape, and the prompt it builds around the gathered context — the genuine per-verb content, with
/// the flow around it living in [`run_synthesis`].
pub struct Verb {
    name: &'static str,
    /// The one-line `--help` description, single-sourced from [`cli`]'s `VERB_*`, so the TUI palette
    /// shows a verb's hint from the verb itself rather than a parallel lookup.
    about: &'static str,
    structured: Option<Structured>,
    prompt: fn(&str) -> String,
}

/// A structured verb's output shape: the item schema, its one-line note, and its kind taxonomy — the
/// three things that differ between structured verbs (assembled into a [`Structure`] per run).
struct Structured {
    item: &'static str,
    note: &'static str,
    kinds: &'static [(&'static str, &'static str)],
}

impl Verb {
    /// This verb's CLI subcommand name (the `arc run <name>` the TUI palette spawns).
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// This verb's one-line `--help` description, for the TUI palette hint.
    pub fn about(&self) -> &'static str {
        self.about
    }

    /// Build this verb's [`Structure`] (if any) and hand it, with the verb's name and prompt, to the
    /// shared synthesis flow.
    pub fn run(&self, args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
        let structure = self.structured.as_ref().map(|s| Structure {
            schema: crate::synth::results_schema(s.item),
            note: s.note,
            kinds: s.kinds,
        });
        run_synthesis(args, global, self.name, structure, self.prompt)
    }
}

// ---- summarize ----

fn summarize_prompt(ctx: &str) -> String {
    format!(
        "You are assessing a code repository from the context below.\n\n\
         {ctx}\n\
         In 3-5 sentences, give a concise, useful assessment: what kind of project it appears \
         to be, its apparent stack, and anything notable or worth a closer look."
    )
}

pub const SUMMARIZE: Verb = Verb {
    name: cli::NAME_SUMMARIZE,
    about: cli::VERB_SUMMARIZE,
    structured: None,
    prompt: summarize_prompt,
};

// ---- suggest ----

/// The `suggest` structured-output item: one suggestion with its rationale.
const SUGGEST_ITEM: &str = r#"{"type":"object","properties":{"suggestion":{"type":"string"},"rationale":{"type":"string"}},"required":["suggestion","rationale"]}"#;

/// Suggest's attention taxonomy (the (label, description) dual use: see [`Structure`]'s `kinds`).
const SUGGEST_KINDS: &[(&str, &str)] = &[
    ("risk", "something fragile or hazardous worth hardening"),
    (
        "improvement",
        "working code or docs that could be clearer or simpler",
    ),
    ("unfinished", "something started but not yet complete"),
    ("verification", "an assumption or claim worth confirming"),
    ("awareness", "context worth knowing, with no action implied"),
];

fn suggest_prompt(ctx: &str) -> String {
    format!(
        "You are reviewing a code repository to advise where attention is best spent. Consider \
         these kinds of suggestion:\n{}\n\n\
         {ctx}\n\
         Produce concrete suggestions, each a short line with a one-clause rationale.",
        kind_list(SUGGEST_KINDS)
    )
}

pub const SUGGEST: Verb = Verb {
    name: cli::NAME_SUGGEST,
    about: cli::VERB_SUGGEST,
    structured: Some(Structured {
        item: SUGGEST_ITEM,
        note: "one object per suggestion.",
        kinds: SUGGEST_KINDS,
    }),
    prompt: suggest_prompt,
};

// ---- extract ----

/// The `extract` structured-output item: one proposed rule.
const EXTRACT_ITEM: &str = r#"{"type":"object","properties":{"id":{"type":"string"},"rule":{"type":"string"},"provenance":{"type":"string"}},"required":["id","rule","provenance"]}"#;

fn extract_prompt(ctx: &str) -> String {
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
}

pub const EXTRACT: Verb = Verb {
    name: cli::NAME_EXTRACT,
    about: cli::VERB_EXTRACT,
    structured: Some(Structured {
        item: EXTRACT_ITEM,
        note: "one object per proposed rule.",
        kinds: &[], // no fixed taxonomy; --kinds lets the model label freely
    }),
    prompt: extract_prompt,
};

// ---- audit ----

/// The `audit` structured-output item: one concrete rule violation.
const AUDIT_ITEM: &str = r#"{"type":"object","properties":{"rule":{"type":"string"},"location":{"type":"string"},"reason":{"type":"string"}},"required":["rule","location","reason"]}"#;

fn audit_prompt(ctx: &str) -> String {
    format!(
        "You are auditing a code repository strictly against the rules provided below (listed \
         under \"Rules\").\n\n\
         {ctx}\n\
         Report only concrete violations of those rules. For each: the rule id, the file/location \
         where it occurs, and a one-clause reason it violates. Do not raise general suggestions, \
         and do not mention rules that aren't violated. If no rules are present in the context, \
         there is nothing to audit against — report no violations and say so in your overall read."
    )
}

pub const AUDIT: Verb = Verb {
    name: cli::NAME_AUDIT,
    about: cli::VERB_AUDIT,
    structured: Some(Structured {
        item: AUDIT_ITEM,
        note: "one object per violation.",
        kinds: &[], // violations already bucket by their `rule`
    }),
    prompt: audit_prompt,
};

// ---- critique ----

/// The `critique` structured-output item: one defect and where it is.
const CRITIQUE_ITEM: &str = r#"{"type":"object","properties":{"location":{"type":"string"},"defect":{"type":"string"}},"required":["location","defect"]}"#;

/// Critique's defect taxonomy (the (label, description) dual use: see [`Structure`]'s `kinds`).
const CRITIQUE_KINDS: &[(&str, &str)] = &[
    (
        "redundancy",
        "the same thing stated or built in more than one place",
    ),
    ("inconsistency", "parts that contradict each other"),
    ("staleness", "claims that no longer match reality"),
    ("gap", "missing pieces or unhandled cases"),
    ("dead", "unused or unreachable elements"),
    (
        "tightening",
        "what could be consolidated, restructured, or clarified",
    ),
];

fn critique_prompt(ctx: &str) -> String {
    format!(
        "You are performing a rigorous critical review of a repository and its documentation to \
         surface quality defects of these kinds:\n{}\n\n\
         {ctx}\n\
         Report concrete findings; for each, the specific location and the problem in a clause. \
         Prefer fewer real findings over padding, and call out cross-cutting redundancy explicitly.",
        kind_list(CRITIQUE_KINDS)
    )
}

pub const CRITIQUE: Verb = Verb {
    name: cli::NAME_CRITIQUE,
    about: cli::VERB_CRITIQUE,
    structured: Some(Structured {
        item: CRITIQUE_ITEM,
        note: "one object per defect.",
        kinds: CRITIQUE_KINDS,
    }),
    prompt: critique_prompt,
};

// ---- verify ----

/// The `verify` structured-output item: one verdict on a previously-recorded finding.
const VERIFY_ITEM: &str = r#"{"type":"object","properties":{"id":{"type":"string"},"verdict":{"type":"string","enum":["reproduces","resolved","indeterminate"]},"reason":{"type":"string"}},"required":["id","verdict","reason"]}"#;

fn verify_prompt(ctx: &str) -> String {
    format!(
        "You are re-checking previously-recorded findings against the current state of a code \
         repository. The repository's open findings ledger is included in the context below, each \
         finding under a `## <id>` heading.\n\n\
         {ctx}\n\
         For each finding, judge against the current code whether it STILL reproduces, has been \
         RESOLVED (the code no longer exhibits it), or is INDETERMINATE (the provided context \
         doesn't contain what's needed to tell). Return one result per finding: its `id` (exactly as \
         in its heading), the verdict (reproduces | resolved | indeterminate), and a one-clause \
         reason grounded in the current code. Judge only what the context supports — prefer \
         indeterminate over guessing. If no findings are present, report none and say so."
    )
}

pub const VERIFY: Verb = Verb {
    name: cli::NAME_VERIFY,
    about: cli::VERB_VERIFY,
    structured: Some(Structured {
        item: VERIFY_ITEM,
        note: "one object per finding re-checked.",
        kinds: &[], // verdicts already bucket by their `verdict`
    }),
    prompt: verify_prompt,
};

// ---- evolve ----

/// The `evolve` structured-output item: one radical proposal.
const EVOLVE_ITEM: &str = r#"{"type":"object","properties":{"change":{"type":"string"},"rationale":{"type":"string"}},"required":["change","rationale"]}"#;

fn evolve_prompt(ctx: &str) -> String {
    format!(
        "You are exploring how this repository could radically evolve.\n\n\
         {ctx}\n\
         Propose drastic overhauls, structural reimaginings, and bold directions that would \
         normally go unspoken — challenge the fundamental assumptions, scope, and shape of the \
         project. What would a fresh attempt, unburdened by the current design, do differently? \
         Treat what exists as a point of departure, not a constraint. For each, give the change \
         and why it could be worth it despite seeming extreme."
    )
}

pub const EVOLVE: Verb = Verb {
    name: cli::NAME_EVOLVE,
    about: cli::VERB_EVOLVE,
    structured: Some(Structured {
        item: EVOLVE_ITEM,
        note: "one object per proposed change.",
        kinds: &[], // no fixed taxonomy; --kinds lets the model label freely
    }),
    prompt: evolve_prompt,
};

// ---- aggregate ----

/// The `aggregate` structured-output item: one merged, cross-run item. Recurrence is read off
/// `sources` (its length, or the distinct repos its runs targeted, via the run records) — derived
/// by the consumer, never model-emitted as a separate count that could disagree with the list.
const AGGREGATE_ITEM: &str = r#"{"type":"object","properties":{"statement":{"type":"string"},"sources":{"type":"array","items":{"type":"string"}},"covered_by":{"type":"string"}},"required":["statement","sources","covered_by"]}"#;

fn aggregate_prompt(ctx: &str) -> String {
    format!(
        "You are aggregating the results of prior runs — included in the context below, each under \
         its run id with the command and repository it examined. Judge which items ACROSS the runs \
         express the same underlying principle or issue in substance: wording will differ, so match \
         meaning, never phrasing.\n\n\
         {ctx}\n\
         Merge each group of same-substance items into one: state it once, as sharply as the best \
         of its sources (or sharper), and record every run it drew from. An item appearing in only \
         one run is kept as-is with its single source — recurrence is signal for the reader, not a \
         filter. Where the context also carries active rules, an item an existing rule already \
         expresses is marked covered rather than re-proposed as new. Order the merged items \
         most-shared first."
    )
}

pub const AGGREGATE: Verb = Verb {
    name: cli::NAME_AGGREGATE,
    about: cli::VERB_AGGREGATE,
    structured: Some(Structured {
        item: AGGREGATE_ITEM,
        note: "one object per merged item: `statement` (the single sharpest statement of the shared substance), `sources` (the run ids it drew from), and `covered_by` (the id of an active rule in context that already expresses it, or an empty string when none does).",
        kinds: &[], // no fixed taxonomy; the aggregated runs' own kinds carry through their items
    }),
    prompt: aggregate_prompt,
};

/// Every synthesis verb, in palette presentation order — the registry the TUI's `run` sub-menu derives
/// from, so a new verb appears there automatically rather than needing a parallel hand-kept list.
pub const ALL: &[&Verb] = &[
    &AUDIT, &CRITIQUE, &VERIFY, &SUGGEST, &SUMMARIZE, &EXTRACT, &EVOLVE, &AGGREGATE,
];

/// Resolve a parsed `arc run <verb>` to its registry row + its args — the single decision point over
/// the closed CLI enum, kept in the registry's own file so dispatch can't grow a parallel home
/// elsewhere (`lib.rs` drives whatever this returns). Adding a verb is its clap variant, its `Verb`
/// row, one arm here, and its [`ALL`] entry — the compiler enforces the arm, the parity test below
/// enforces `ALL`.
pub fn resolve(verb: &cli::RunVerb) -> (&'static Verb, &SynthArgs) {
    use cli::RunVerb as V;
    match verb {
        V::Summarize(a) => (&SUMMARIZE, a),
        V::Suggest(a) => (&SUGGEST, a),
        V::Extract(a) => (&EXTRACT, a),
        V::Audit(a) => (&AUDIT, a),
        V::Critique(a) => (&CRITIQUE, a),
        V::Verify(a) => (&VERIFY, a),
        V::Evolve(a) => (&EVOLVE, a),
        V::Aggregate(a) => (&AGGREGATE, a),
    }
}

#[cfg(test)]
mod tests {
    /// The verb set has two compile-checked homes (the clap enum, whose dispatch match won't build
    /// with a missing arm) and one that isn't ([`super::ALL`], the TUI registry). This pins them
    /// together, so a verb added to clap but missed here fails a test instead of silently missing
    /// from the palette.
    #[test]
    fn all_registry_matches_the_clap_verb_subcommands() {
        let cmd = <crate::cli::Cli as clap::CommandFactory>::command();
        let run = cmd
            .find_subcommand(crate::cli::NAME_RUN)
            .expect("the run group exists");
        let clap_names: std::collections::BTreeSet<String> = run
            .get_subcommands()
            .map(|c| c.get_name().to_owned())
            .collect();
        let all_names: std::collections::BTreeSet<String> =
            super::ALL.iter().map(|v| v.name().to_owned()).collect();
        assert_eq!(clap_names, all_names);
    }
}
