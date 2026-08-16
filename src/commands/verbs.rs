//! The AI synthesis verbs, as data. Each verb is the shared synthesis flow ([`run_synthesis`])
//! wrapped around a minimal role directive and an optional structured-output shape; collapsing the
//! near-identical command modules into one table single-sources the build-the-`Structure`-and-
//! run boilerplate, so a change to that shared shape is one edit rather than nine.

use std::process::ExitCode;

use super::{Policy, Structure, run_synthesis};
use crate::cli::{self, GlobalArgs, SynthArgs};

/// One AI verb: its CLI name (single-sourced from [`cli`]'s `NAME_*`), its optional structured-output
/// shape, and its role directive.
pub struct Verb {
    name: &'static str,
    /// The one-line `--help` description, single-sourced from [`cli`]'s `VERB_*`, so the TUI palette
    /// shows a verb's hint from the verb itself rather than a parallel lookup.
    about: &'static str,
    structured: Option<Structured>,
    /// The verb's role directive — the minimal per-verb instruction stating what the judgment is
    /// and what qualifies. Everything else in the prompt is shared assembly in [`run_synthesis`]
    /// (the effective taxonomy block, the gathered context, the grounding guardrail, the
    /// structured-output notes), so the genuinely per-verb content is this data, and no
    /// verb-specific machinery hides in phrasing.
    role: &'static str,
    /// The verb's cross-cutting policy ([`Policy`]) — auto-loaded ledgers and cross-run inputs as
    /// registry data, so the shared flow never branches on a verb's name. Rows state only what
    /// differs from [`Policy::NONE`].
    policy: Policy,
}

/// A structured verb's output shape: the item schema, its item-shape note (the fields and their
/// meaning, appended to the shared structured-output framing), and its built-in kind taxonomy — the
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

    /// Build this verb's [`Structure`] (if any) and hand it, with the verb's name and role, to the
    /// shared synthesis flow.
    pub fn run(&self, args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<ExitCode> {
        let structure = self.structured.as_ref().map(|s| Structure {
            schema: crate::synth::results_schema(s.item),
            note: s.note,
            kinds: s.kinds,
        });
        run_synthesis(args, global, self.name, structure, self.role, self.policy)
    }
}

// ---- summarize ----

pub const SUMMARIZE: Verb = Verb {
    name: cli::NAME_SUMMARIZE,
    about: cli::VERB_SUMMARIZE,
    structured: None,
    role: "You are assessing a code repository from the supplied context. In 3-5 sentences, give a \
           concise, useful assessment: what kind of project it appears to be, its apparent stack, \
           and anything notable or worth a closer look.",
    policy: Policy::NONE,
};

// ---- suggest ----

/// The `suggest` structured-output item: one suggestion with its rationale.
const SUGGEST_ITEM: &str = r#"{"type":"object","properties":{"suggestion":{"type":"string"},"rationale":{"type":"string"}},"required":["suggestion","rationale"]}"#;

/// Suggest's built-in attention taxonomy (see [`Structure`]'s `kinds` for the dual use).
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

pub const SUGGEST: Verb = Verb {
    name: cli::NAME_SUGGEST,
    about: cli::VERB_SUGGEST,
    structured: Some(Structured {
        item: SUGGEST_ITEM,
        note: "one object per suggestion: the concrete `suggestion` and its one-clause `rationale`.",
        kinds: SUGGEST_KINDS,
    }),
    role: "You are reviewing a code repository to advise where attention is best spent.",
    policy: Policy::NONE,
};

// ---- extract ----

/// The `extract` structured-output item: one proposed rule.
const EXTRACT_ITEM: &str = r#"{"type":"object","properties":{"id":{"type":"string"},"rule":{"type":"string"},"provenance":{"type":"string"}},"required":["id","rule","provenance"]}"#;

pub const EXTRACT: Verb = Verb {
    name: cli::NAME_EXTRACT,
    about: cli::VERB_EXTRACT,
    structured: Some(Structured {
        item: EXTRACT_ITEM,
        note: "one object per proposed rule: a short kebab-case `id`, the `rule` as one tight \
               paragraph stating the principle or anti-pattern and how to recognize it (included \
               verbatim into future runs), and its `provenance` (where in this repo it came from).",
        kinds: &[], // no built-in taxonomy; --kinds lets the model label freely
    }),
    role: "You are extracting reusable engineering rules from a code repository — coding \
           standards, anti-patterns, and principles that generalize beyond this one repo. Favor \
           what the code actually evidences over generic advice, and treat any rules already \
           present in the context as existing policy not to duplicate. Propose only rules that \
           clearly earn their place — never pad toward a count.",
    policy: Policy::NONE,
};

// ---- audit ----

/// The `audit` structured-output item: one concrete rule violation.
const AUDIT_ITEM: &str = r#"{"type":"object","properties":{"rule":{"type":"string"},"location":{"type":"string"},"reason":{"type":"string"}},"required":["rule","location","reason"]}"#;

pub const AUDIT: Verb = Verb {
    name: cli::NAME_AUDIT,
    about: cli::VERB_AUDIT,
    structured: Some(Structured {
        item: AUDIT_ITEM,
        note: "one object per violation: the `rule` id, the `location` where it occurs, and a \
               one-clause `reason` it violates.",
        kinds: &[], // violations already bucket by their `rule`
    }),
    role: "You are auditing a code repository strictly against the rules provided in the context. \
           Report only concrete violations of those rules — no general suggestions, and no mention \
           of rules that are not violated. If no rules are present in the context, there is \
           nothing to audit against.",
    policy: Policy::NONE,
};

// ---- critique ----

/// The `critique` structured-output item: one defect and where it is.
const CRITIQUE_ITEM: &str = r#"{"type":"object","properties":{"location":{"type":"string"},"defect":{"type":"string"}},"required":["location","defect"]}"#;

/// Critique's built-in defect taxonomy (see [`Structure`]'s `kinds` for the dual use).
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

pub const CRITIQUE: Verb = Verb {
    name: cli::NAME_CRITIQUE,
    about: cli::VERB_CRITIQUE,
    structured: Some(Structured {
        item: CRITIQUE_ITEM,
        note: "one object per defect: the specific `location` and the `defect` in a clause.",
        kinds: CRITIQUE_KINDS,
    }),
    role: "You are performing a rigorous critical review of a repository and its documentation. \
           Report concrete defects — prefer fewer real findings over padding.",
    policy: Policy::NONE,
};

// ---- verify ----

/// The `verify` structured-output item: one verdict on a previously-recorded finding.
const VERIFY_ITEM: &str = r#"{"type":"object","properties":{"id":{"type":"string"},"verdict":{"type":"string","enum":["reproduces","resolved","indeterminate"]},"reason":{"type":"string"}},"required":["id","verdict","reason"]}"#;

pub const VERIFY: Verb = Verb {
    name: cli::NAME_VERIFY,
    about: cli::VERB_VERIFY,
    structured: Some(Structured {
        item: VERIFY_ITEM,
        note: "one object per finding re-checked: its `id` exactly as in its heading, the \
               `verdict` (reproduces | resolved | indeterminate), and a one-clause `reason` \
               grounded in the current code.",
        kinds: &[], // verdicts already bucket by their `verdict`
    }),
    role: "You are re-checking previously-recorded findings — the repository's open findings \
           ledger in the context, each finding under a `## <id>` heading — against the current \
           state of the code. Judge each strictly by what the context supports: it still \
           reproduces, it is resolved (the code no longer exhibits it), or it is indeterminate \
           (the provided context does not contain what is needed to tell) — prefer indeterminate \
           over guessing.",
    policy: Policy {
        recheck_findings: true,
        ..Policy::NONE
    },
};

// ---- evolve ----

/// The `evolve` structured-output item: one radical proposal.
const EVOLVE_ITEM: &str = r#"{"type":"object","properties":{"change":{"type":"string"},"rationale":{"type":"string"}},"required":["change","rationale"]}"#;

pub const EVOLVE: Verb = Verb {
    name: cli::NAME_EVOLVE,
    about: cli::VERB_EVOLVE,
    structured: Some(Structured {
        item: EVOLVE_ITEM,
        note: "one object per proposed change: the `change` and its `rationale` — why it could be \
               worth it despite seeming extreme.",
        kinds: &[], // no built-in taxonomy; --kinds lets the model label freely
    }),
    role: "You are exploring how this repository could radically evolve. Propose the drastic \
           overhauls, structural reimaginings, and bold directions that would normally go unspoken \
           — challenge the fundamental assumptions, scope, and shape of the project, and treat \
           what exists as a point of departure, not a constraint.",
    policy: Policy::NONE,
};

// ---- aggregate ----

/// The `aggregate` structured-output item: one merged, cross-run item. Recurrence is read off
/// `sources` (its length, or the distinct repos its runs targeted, via the run records) — derived
/// by the consumer, never model-emitted as a separate count that could disagree with the list.
const AGGREGATE_ITEM: &str = r#"{"type":"object","properties":{"statement":{"type":"string"},"sources":{"type":"array","items":{"type":"string"}},"covered_by":{"type":"string"}},"required":["statement","sources","covered_by"]}"#;

pub const AGGREGATE: Verb = Verb {
    name: cli::NAME_AGGREGATE,
    about: cli::VERB_AGGREGATE,
    structured: Some(Structured {
        item: AGGREGATE_ITEM,
        note: "one object per merged item: `statement` (the single sharpest statement of the \
               shared substance), `sources` (the run ids it drew from), and `covered_by` (the id \
               of an active rule in context that already expresses it, or an empty string when \
               none does).",
        kinds: &[], // no built-in taxonomy; the aggregated runs' own kinds carry through their items
    }),
    role: "You are aggregating the results of prior runs — included in the context, each under its \
           run id with the command and repository it examined. Judge which items across the runs \
           express the same substance — wording will differ, so match meaning, never phrasing — \
           and merge each same-substance group into one item, stated as sharply as the best of its \
           sources or sharper. Keep an item appearing in only one run as-is: recurrence is signal \
           for the reader, not a filter. Where the context also carries active rules, mark an item \
           an existing rule already expresses as covered rather than re-proposing it. Order the \
           merged items most-shared first.",
    policy: Policy {
        consumes_from: true,
        ..Policy::NONE
    },
};

// ---- align ----

/// The `align` structured-output item: one finding over the tracked items. `items` names the ids
/// involved — `(order)` for the order file, `(repo)` for repository state — so a finding is always
/// addressable to the material it grounds in.
const ALIGN_ITEM: &str = r#"{"type":"object","properties":{"kind":{"type":"string"},"items":{"type":"array","items":{"type":"string"}},"reason":{"type":"string"}},"required":["kind","items","reason"]}"#;

/// Align's built-in taxonomy — each kind names a way the agenda can fail, judged from the supplied
/// material: the items, their intended order, and the repository state. The kinds' definitions are
/// where the judgment's scope lives (the built-in default, overridable like any taxonomy), and
/// which of them prove reliably agreeable is the firing record's question, settled by exercise.
const ALIGN_KINDS: &[(&str, &str)] = &[
    (
        "contradiction",
        "incompatible statements — within one item, between items, or between an item and the repository state in context",
    ),
    (
        "redundancy",
        "items that substantially duplicate one another and belong merged",
    ),
    (
        "disorder",
        "ordering, organization, or planning at odds with what the supplied material shows — a dependency or prerequisite the sequence ignores, stated or evident",
    ),
    (
        "staleness",
        "an item overtaken by the repository state in context — already landed, or its premise gone",
    ),
    (
        "ambiguity",
        "an item too underspecified to act on as written",
    ),
    (
        "verbosity",
        "filler that dictates or explains beyond what acting on the item needs",
    ),
    (
        "irrelevance",
        "content that does not belong to the item or the agenda — noise adding nothing",
    ),
    (
        "disparity",
        "unevenness across the set — items diverging in structure, format, or depth without cause (depth proportionate to an item's scope is caused, not uneven)",
    ),
];

pub const ALIGN: Verb = Verb {
    name: cli::NAME_ALIGN,
    about: cli::VERB_ALIGN,
    structured: Some(Structured {
        item: ALIGN_ITEM,
        note: "one object per finding: the `kind`, the `items` involved (item ids; `(order)` for \
               the order file, `(repo)` for repository state), and a one-clause `reason` grounded \
               in the context.",
        kinds: ALIGN_KINDS,
    }),
    role: "You are auditing a repository's tracked items — its open agenda, included in the \
           context with its intended order — against each other and against the supplied \
           repository state.",
    policy: Policy {
        agenda: true,
        ..Policy::NONE
    },
};

/// Every synthesis verb, in palette presentation order — the registry the TUI's `run` sub-menu derives
/// from, so a new verb appears there automatically rather than needing a parallel hand-kept list.
pub const ALL: &[&Verb] = &[
    &AUDIT, &CRITIQUE, &VERIFY, &ALIGN, &SUGGEST, &SUMMARIZE, &EXTRACT, &EVOLVE, &AGGREGATE,
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
        V::Align(a) => (&ALIGN, a),
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
