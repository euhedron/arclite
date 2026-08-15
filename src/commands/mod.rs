pub mod config;
pub mod doctor;
pub mod init;
pub mod inspect;
pub mod log;
pub mod models;
pub mod promote;
pub mod retire;
pub mod rules;
pub mod status;
pub mod tui;
pub mod update;
pub mod usage;
pub mod verbs;

use std::process::ExitCode;

use anyhow::Context;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::synth::{self, SynthOptions};

/// A command's structured output: a JSON Schema the model's result is validated against (returned
/// as `structured_output`), plus a note describing its item shape (appended to the shared
/// [`STRUCTURED_NOTE`] framing).
/// The schema is the shared `results`-array envelope ([`crate::synth::results_schema`]) wrapping the
/// command's own item shape — so commands declare only what differs. A verb that declares one always
/// produces it — the typed result is the substrate, human text a rendering of it — and only
/// `summarize` declares none (its whole point is prose). The gate, `--ranked`, `--kinds`, and
/// multi-run aggregation all treat the `results` array uniformly; `--fail-on-findings` blocks when
/// it's non-empty.
pub struct Structure {
    pub schema: String,
    pub note: &'static str,
    /// The verb's built-in kind taxonomy as (label, description) pairs — the shipped default that
    /// the `taxonomies` setting overrides or extends per verb ([`resolve_taxonomy`] merges by
    /// label). The *effective* set is listed in the assembled prompt as the substance of what the
    /// verb looks for, recorded on the run as (kind, hash) pairs, and reused by `--kinds` as the
    /// suggested classification vocabulary. Empty = no built-in taxonomy (`--kinds` then lets the
    /// model label freely, unless settings supply one).
    pub kinds: &'static [(&'static str, &'static str)],
}

/// Grounding guardrail appended to every synthesis prompt (single-sourced, not restated per prompt).
const GROUNDING: &str = "\n\nGround everything you report in the context above; include nothing you cannot point to in it. For version-control claims, obey the Version-control truth block: a path reference is not evidence that the referenced path exists, is tracked, or is committed.";

/// Appended by `--ranked`: order the results by significance (the array order is the ranking).
const RANKED_NOTE: &str =
    "\n\nOrder the results from most to least significant; the order is the ranking.";

/// Shared framing for structured output, prepended to the command's own item-shape note
/// (single-sourced like [`GROUNDING`]/[`RANKED_NOTE`], so it can't drift between commands).
const STRUCTURED_NOTE: &str = "\n\nReturn the result as structured data — ";

/// Appended after the command's item-shape note: every structured run also returns a required
/// top-level `note`, so an empty `results` is a judged outcome rather than silence. The note also
/// carries what a prose report would have said around the findings — notably anything weighed but
/// deliberately not raised — so the structured channel loses none of the judgment's edges.
const NOTE_INSTRUCTION: &str = " Also include a top-level `note`: one or two clauses giving the overall read of the run (what was assessed, and the upshot) — especially when `results` is empty — plus anything you weighed but deliberately did not raise, so the judgment's edges stay visible.";

/// Bridges the role directive to the effective taxonomy in the assembled prompt — the kinds are
/// the substance of what a taxonomy-bearing verb looks for (see [`Structure`]'s `kinds`).
const KINDS_HEADER: &str = "\n\nThe kinds of finding to report:\n";

/// Render a taxonomy as a labelled list — `- label: description` per line — for the assembled
/// prompt.
fn kind_list(kinds: &[(String, String)]) -> String {
    kinds
        .iter()
        .map(|(label, description)| format!("- {label}: {description}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolve a verb's effective taxonomy: the built-in default with the `taxonomies` settings entry
/// for this verb merged over it, later winning by kind label — the rules model applied to the
/// vocabulary lever, so an operator extends the shipped set with new kinds or overrides a shipped
/// kind's description by restating its label, and the prompt only ever carries the one resolved
/// vocabulary (resolution happens in data, never as an in-prompt instruction to prefer one list
/// over another). Returns the effective (label, description) pairs plus how many entries settings
/// contributed (0 = built-in as shipped), for the sources disclosure.
fn resolve_taxonomy(
    builtin: &[(&str, &str)],
    configured: Option<&Vec<(String, String)>>,
) -> (Vec<(String, String)>, usize) {
    let mut effective: Vec<(String, String)> = builtin
        .iter()
        .map(|&(label, description)| (label.to_owned(), description.to_owned()))
        .collect();
    let configured = configured.map_or(&[][..], |v| v.as_slice());
    for (label, description) in configured {
        if let Some(slot) = effective.iter_mut().find(|(l, _)| l == label) {
            slot.1 = description.clone();
        } else {
            effective.push((label.clone(), description.clone()));
        }
    }
    (effective, configured.len())
}

/// Appended by `--kinds`: ask for a per-item `kind`. With a taxonomy in play the schema enum-locks
/// the field to it ([`synth::lock_kinds`]), so the note simply names the vocabulary; under
/// `--free-kinds` the lock is off and the model may label outside it — deviation as deliberate,
/// recorded opt-in signal about the taxonomy's fit (a poor fit is also the note field's to say).
/// With no taxonomy, it labels freely. Like `--ranked`, this shapes the output in any mode; the
/// classification is the lever's, never a command's prompt.
fn kinds_note(has_taxonomy: bool, free: bool) -> &'static str {
    match (has_taxonomy, free) {
        (true, false) => "\n\nAlso give each result a `kind` — one of the kinds listed above.",
        (true, true) => {
            "\n\nAlso give each result a `kind` — one of the kinds listed above, or your own if none fit."
        }
        (false, _) => "\n\nAlso give each result a `kind` — its category of finding.",
    }
}

/// Shared flow for the AI synthesis commands: gather the repo context once, assemble the prompt
/// around the verb's role directive (role, effective taxonomy, context, grounding, output notes),
/// then run — so the commands can't drift in how they wire context, tools, the granted dir, cost
/// reporting, or structured output. `structure` is the command's structured output (see
/// [`Structure`]), always active when declared; `--fail-on-findings` requires one.
pub fn run_synthesis(
    args: &SynthArgs,
    global: &GlobalArgs,
    command: &str,
    structure: Option<Structure>,
    role: &str,
) -> anyhow::Result<ExitCode> {
    anyhow::ensure!(
        (1..=crate::synth::MAX_RUNS).contains(&args.runs),
        "--runs must be between 1 and {}, got {}",
        crate::synth::MAX_RUNS,
        args.runs
    );
    // verify auto-loads the open ledger framed for re-checking — the opposite framing of
    // --findings' "surface new issues beyond these" — so the flag is rejected rather than
    // silently overridden.
    anyhow::ensure!(
        !(args.findings && command == crate::cli::NAME_VERIFY),
        "`{command}` already re-checks the open findings ledger — drop --findings"
    );
    // --from feeds prior runs' results as context; only aggregate consumes it, and its judgment —
    // sameness ACROSS runs — needs at least two. Both mismatches rejected before spend, never
    // silently ignored.
    if command == crate::cli::NAME_AGGREGATE {
        anyhow::ensure!(
            args.from.len() >= 2,
            "`{command}` merges results across runs — name at least two with --from <run-id>"
        );
    } else {
        anyhow::ensure!(
            args.from.is_empty(),
            "--from feeds prior runs to `aggregate` — `{command}` doesn't consume it"
        );
    }
    let settings = crate::settings::Settings::load(&args.path)?;
    // align judges the agenda, not the code against standards: the configured/default ruleset is
    // not auto-loaded (the rules block's weigh-the-repository-against framing would misdirect an
    // agenda judgment, at real token cost per gate round) — rules join an align run only by
    // explicit --rules/--ruleset, disclosed like any selection.
    let resolution =
        if command == crate::cli::NAME_ALIGN && args.rules.is_none() && args.ruleset.is_none() {
            RuleResolution {
                description: "none (align audits the agenda; --ruleset composes rules in)"
                    .to_owned(),
                sources: Vec::new(),
            }
        } else {
            resolve_rule_sources(args.rules.as_deref(), args.ruleset.as_deref(), &settings)?
        };
    // Backend: the `--backend` flag over the configured `backend` setting — and nothing beneath
    // (no built-in default: the backend is the thing that spends, so an unselected one errors with
    // what's detected rather than silently picking a vendor). The resolved instance owns the
    // per-backend policy below — which model default applies, whether a native spend cap is
    // honored, and which requested capabilities it can't — so this function never branches on the
    // backend name (that lives only in `ai::backend`, the single home of the known backends).
    let backend_name = args
        .backend
        .clone()
        .or_else(|| settings.backend.clone())
        .ok_or_else(crate::ai::no_backend_selected)?;
    let backend = crate::ai::backend(&backend_name)?;
    // Validate an explicit cap at the boundary — the same rule `config set` and the settings loader
    // apply — so a zero/negative/non-finite value is rejected before any spend rather than riding
    // into the backend as a nonsense "safety" cap.
    if let Some(cap) = args.max_budget_usd {
        crate::settings::validate_budget(cap).context("invalid --max-budget-usd")?;
    }
    // Reject, before any spend, a requested capability this backend can't honor — surfaced as an
    // error, never silently dropped.
    backend.reject_unsupported(args.max_budget_usd, &args.allow_tool)?;
    let model = backend.resolve_model(args.model.as_deref(), backend.configured_model(&settings));
    // The resolved id — whichever of flag/config/default supplied it — rides argv as `--model`'s
    // value; reject an option-shaped or empty one here, before any spend, rather than let it
    // escape its value slot in the child CLI's grammar.
    crate::ai::validate_model_id(&model)?;
    let max_budget_usd = backend.resolve_budget(args.max_budget_usd, settings.max_budget_usd);
    // A configured budget cap the backend can't honor is surfaced, never silently dropped. An explicit
    // --max-budget-usd is already rejected above; a configured default would otherwise just vanish here
    // (e.g. codex has no native cap), leaving the user's safety lever silently inactive.
    if args.max_budget_usd.is_none()
        && let Some(cap) = settings.max_budget_usd
        && max_budget_usd.is_none()
    {
        eprintln!(
            "arclite: the max_budget_usd setting ({}) not applied — the {backend_name} backend has no native budget cap",
            crate::log::cost_display(cap)
        );
    }
    let reasoning_effort = backend.reasoning_effort(settings.codex_reasoning_effort.as_deref());
    let log = settings.logging_enabled();
    // Disclose which settings layers are active (user then project) in the run output — configuration
    // detected and in effect is reported, never left for the reader to infer.
    let config = settings.active_display();
    let mut ctx = synth::gather_context(
        &args.path,
        &synth::ContextSpec {
            includes: &args.include,
            rule_sources: &resolution.sources,
            disabled_rules: &settings.disabled_rules,
            max: args.max_file_chars,
            changed: args.changed,
            exclude: &args.exclude,
            scan: !args.no_scan,
            findings: args.findings,
            // verify auto-loads the open ledger framed for re-checking (--findings rejected above)
            recheck_findings: command == crate::cli::NAME_VERIFY,
            // align auto-loads the tracked items + their intended order — its whole subject
            agenda: command == crate::cli::NAME_ALIGN,
            from_runs: &args.from,
        },
    )?;
    // The effective taxonomy: settings over built-in, merged by label — resolved as data before
    // the prompt exists, so the model only ever sees one vocabulary and the run records which
    // (the (kind, hash) pairs on the record). A settings contribution is disclosed as a source
    // like any other lever in play.
    let (kinds, kinds_from_settings) = resolve_taxonomy(
        structure.as_ref().map_or(&[][..], |s| s.kinds),
        settings.taxonomies.get(command),
    );
    if kinds_from_settings > 0 {
        ctx.sources.push(format!(
            "taxonomy: {} kinds — {kinds_from_settings} from settings, merged over built-in by label",
            kinds.len()
        ));
    }
    let taxonomy: Vec<synth::ActiveKind> = kinds
        .iter()
        .map(|(kind, description)| synth::ActiveKind {
            kind: kind.clone(),
            hash: crate::rules::fingerprint(description),
        })
        .collect();
    let mut prompt = role.to_owned();
    if !kinds.is_empty() {
        prompt.push_str(KINDS_HEADER);
        prompt.push_str(&kind_list(&kinds));
    }
    prompt.push_str("\n\n");
    prompt.push_str(&ctx.text);
    prompt.push_str(GROUNDING);
    // A verb that declares a structured shape always produces it: the typed `results` are the
    // canonical output everything downstream acts on (the gate, promote, multi-run union, ranking),
    // and the human view derives from them — prose-as-product remains only for verbs without a
    // structure (summarize), where narrative is the deliverable. --fail-on-findings additionally
    // gates on the results; it still requires a structure, so a prose verb rejects it rather than
    // silently ignoring it.
    let gate = if args.fail_on_findings {
        anyhow::ensure!(
            structure.is_some(),
            "`{command}` has no structured output to gate on — drop --fail-on-findings"
        );
        // Gate on the `results` array the schemas produce — the key single-sourced in synth.
        Some(crate::synth::RESULTS_KEY)
    } else {
        None
    };
    let schema = if let Some(s) = &structure {
        prompt.push_str(STRUCTURED_NOTE);
        prompt.push_str(s.note);
        prompt.push_str(NOTE_INSTRUCTION);
        // --kinds adds a per-item `kind`; with a taxonomy in play the field is enum-locked to it
        // below, so membership is the provider's schema guarantee, not a prompt hope.
        let base = if args.kinds {
            synth::with_kind(&s.schema)
        } else {
            s.schema.clone()
        };
        let locked = synth::lock_kinds(&base, &kinds);
        // --free-kinds must actually free something — a `kind` the taxonomy would otherwise lock;
        // anything less is a no-op flag, rejected rather than silently carried.
        if args.free_kinds {
            anyhow::ensure!(
                locked.is_some(),
                "--free-kinds unlocks a taxonomy-locked `kind`, and `{command}` has {} — drop --free-kinds",
                if kinds.is_empty() {
                    "no taxonomy in play (`kind` is already free)"
                } else {
                    "no per-item `kind` field (add --kinds)"
                }
            );
        }
        Some(if args.free_kinds {
            base
        } else {
            locked.unwrap_or(base)
        })
    } else {
        anyhow::ensure!(
            !args.free_kinds,
            "--free-kinds shapes structured output — `{command}` has none"
        );
        None
    };
    // --kinds and --ranked shape the output in any mode (a prompt note; structured runs also carry
    // it in the `kind` field / array order above) — neither requires structured output.
    if args.kinds {
        prompt.push_str(kinds_note(!kinds.is_empty(), args.free_kinds));
    }
    if args.ranked {
        prompt.push_str(RANKED_NOTE);
    }
    let outcome = synth::run(
        &prompt,
        &SynthOptions {
            model: &model,
            backend: &backend_name,
            runs: args.runs,
            max_budget_usd,
            reasoning_effort: reasoning_effort.as_deref(),
            ranked: args.ranked,
            kinds: args.kinds,
            free_kinds: args.free_kinds,
            allowed_tools: &args.allow_tool,
            dir: &ctx.root,
            sources: &ctx.sources,
            rules_active: &ctx.rules_active,
            taxonomy: &taxonomy,
            excluded: &ctx.excluded,
            config: &config,
            command,
            output: args.output.as_deref(),
            ambient_memory: args.ambient_memory,
            schema: schema.as_deref(),
            gate,
            dry_run: args.dry_run,
            json: global.json,
            log,
        },
    );
    // A backend that fails (e.g. codex hitting its workspace spend cap, or an unavailable model) is
    // often recoverable by switching, so name the other available backends rather than leaving the
    // user to recall them. Only on a hard error — a logged errored run (a tripped per-run budget the
    // user set) is the user's own cap, not a backend to switch away from.
    if outcome.is_err() {
        let others: Vec<&str> = crate::ai::known_backends()
            .iter()
            .copied()
            .filter(|&b| b != backend_name.as_str())
            .collect();
        if let Some(first) = others.first() {
            eprintln!(
                "arclite: the {backend_name} backend failed — other backends available: {} (switch with --backend {first})",
                others.join(", ")
            );
        }
    }
    outcome
}

/// What `--rules`/`--ruleset`/the `ruleset` setting resolved to: a human description of the
/// selection (for reporting) plus the source paths to load. Shared by `run_synthesis` and `arc rules`.
pub(crate) struct RuleResolution {
    pub description: String,
    pub sources: Vec<std::path::PathBuf>,
}

/// Resolve which rule sources to load, in precedence order: an ad-hoc `--rules <path>`, else a
/// named `--ruleset <id>` (or the configured `ruleset` setting), else none.
pub(crate) fn resolve_rule_sources(
    rules: Option<&std::path::Path>,
    ruleset: Option<&str>,
    settings: &crate::settings::Settings,
) -> anyhow::Result<RuleResolution> {
    if let Some(path) = rules {
        return Ok(RuleResolution {
            description: format!("ad-hoc rules: {}", path.display()),
            sources: vec![path.to_path_buf()],
        });
    }
    let from_flag = ruleset.is_some();
    let Some(id) = ruleset.or(settings.ruleset.as_deref()) else {
        return Ok(RuleResolution {
            description: "no ruleset selected".to_owned(),
            sources: Vec::new(),
        });
    };
    let origin = if from_flag {
        "--ruleset"
    } else {
        "the `ruleset` setting"
    };
    let Some(sources) = settings.ruleset_sources(id) else {
        // The reserved `default` always resolves: undefined in settings, it is the built-in
        // ruleset arc ships (a project defining its own `default` takes the branch above —
        // the override is theirs by design, and the description names which one resolved).
        if id == crate::DEFAULT_RULESET {
            return Ok(RuleResolution {
                description: format!(
                    "ruleset `{id}` (from {origin}; built-in — the rules arc v{} ships)",
                    env!("CARGO_PKG_VERSION")
                ),
                sources: vec![std::path::PathBuf::from(crate::rules::BUILTIN_SOURCE)],
            });
        }
        anyhow::bail!("ruleset `{id}` is not defined in .arc/settings.json");
    };
    Ok(RuleResolution {
        description: if id == crate::DEFAULT_RULESET {
            format!("ruleset `{id}` (from {origin}; project-defined, overriding the built-in)")
        } else {
            format!("ruleset `{id}` (from {origin})")
        },
        sources: sources.to_vec(),
    })
}

/// Resolve `path` to an absolute path with a uniform error — shared by the command entry points, so
/// the resolution and its wording are single-sourced rather than copy-pasted.
pub(crate) fn resolve_root(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let root =
        std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))?;
    // Validate once, at the boundary: run records, markers, and ledger entries all carry the repo
    // path as JSON text a later command reopens, so a non-UTF-8 root would have to ride lossily —
    // stored state silently addressing a different path. Rejecting here makes every downstream
    // conversion exact by construction (see log::repo_record_string).
    anyhow::ensure!(
        root.to_str().is_some(),
        "arclite needs a UTF-8 repository path — {} can't be recorded and reopened losslessly",
        root.display()
    );
    Ok(root)
}
