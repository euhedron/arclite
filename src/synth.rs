//! Shared synthesis runner for AI-backed commands (`summarize`, `suggest`, …).
//!
//! `--dry-run` previews the prompt + estimate at zero spend; real calls report actual cost + cache
//! usage; and every run echoes the full parameter set it used (model, tools, context sources).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use serde::Serialize;

use crate::ai;
use crate::output::emit;

/// The ceiling on `--runs`. Each run is a full, concurrent agent-CLI process at real per-run cost, so
/// an unbounded count would run away on both load and spend. Kept modest — generous for consensus
/// sampling, low enough that even the max isn't wasteful at a premium model's price. Enforced in
/// `run_synthesis`.
pub(crate) const MAX_RUNS: usize = 8;

const DRY_RUN_NOTE: &str = "estimate counts the prompt only; a real call also loads the model's base system/tool context (not counted here) — actual usage is reported after the call runs";

/// Exit code when an opt-in gate (`--fail-on-findings`) blocks — distinct from `1` (arclite error)
/// so a hook/CI can tell "found violations" apart from "the tool failed". Any non-zero blocks.
/// Also formatted into the `arc --help` exit-code section (see `cli::exit_codes_help`).
pub(crate) const GATE_BLOCKED_EXIT: u8 = 2;

/// Exit code for a run that *errored* — the agent reported a failure (e.g. a tripped budget cap)
/// mid-run. Distinct from a gate block (a clean run with findings, [`GATE_BLOCKED_EXIT`]) and from
/// success, so a hook or script can tell "the run broke" from "the run ran and found problems".
const ERRORED_EXIT: u8 = 1;

const LOGGING_OFF_NOTE: &str = "\nlogging: off (the `logging` setting is false)";

/// The single key for every command's structured output: a `results` array. Defined once — the
/// schema is built from it ([`results_schema`]), the gate reads it, and multi-run unions it — so it
/// can't drift across schema and code.
pub(crate) const RESULTS_KEY: &str = "results";

/// The required companion to [`RESULTS_KEY`] in every structured envelope: a free-text overall read
/// of the run. It makes an empty `results` a *judged* outcome — the model ran and found nothing —
/// rather than indistinguishable from silence. Commentary only; the gate never reads it.
pub(crate) const NOTE_KEY: &str = "note";

/// The per-item classification field `--kinds` adds: each result carries its category of finding.
/// Commentary/bucketing only — the gate never reads it.
pub(crate) const KIND_KEY: &str = "kind";

/// Add the `--kinds` field to a results schema: each item gains a required free-string [`KIND_KEY`] —
/// free-form, *not* a hard enum, so the model isn't locked to the command's suggested taxonomy. (The
/// prompt does the suggesting and interpreting; that rationale lives in `commands`' `kinds_note`.)
pub(crate) fn with_kind(schema: &str) -> String {
    // This schema is one the code itself built from a const item via [`results_schema`], so its shape
    // — valid JSON, a `results.items` object carrying `properties` + `required` — is a code-held
    // invariant; assert it rather than threading recoverable errors over a navigation that can't fail.
    let mut root: serde_json::Value =
        serde_json::from_str(schema).expect("a command's results schema is valid JSON");
    let item = root
        .pointer_mut(&format!("/properties/{RESULTS_KEY}/items"))
        .expect("a results schema declares an items shape");
    item.pointer_mut("/properties")
        .and_then(serde_json::Value::as_object_mut)
        .expect("a results item schema has properties")
        .insert(KIND_KEY.to_owned(), serde_json::json!({ "type": "string" }));
    item.pointer_mut("/required")
        .and_then(serde_json::Value::as_array_mut)
        .expect("a results item schema has a required list")
        .push(KIND_KEY.into());
    root.to_string()
}

/// Wrap a command's array-item schema in the shared `{ results: [ <item> ], note }` envelope, so
/// each command declares only its item shape. The CLI's structured output requires a root object (a
/// top-level array is rejected — confirmed by exercise), so the list can't be the root. Every object
/// is then closed (`additionalProperties: false`): codex's structured output requires it (confirmed
/// by exercise — it 400s otherwise), claude accepts it, so it lives here once rather than in each
/// command's item schema.
/// Enum-lock the per-item `kind` to the effective taxonomy's labels — membership becomes the
/// provider's constrained-decoding guarantee (the mechanism verify's `verdict` already rests on),
/// so the run's recorded taxonomy provably covers every finding's `kind`. Returns `None` when there
/// is nothing to lock: no taxonomy in play, or no per-item `kind` in the schema. (`--free-kinds`
/// skips the lock deliberately; the flag rides the record.)
pub(crate) fn lock_kinds(schema: &str, kinds: &[(String, String)]) -> Option<String> {
    if kinds.is_empty() {
        return None;
    }
    let mut root: serde_json::Value =
        serde_json::from_str(schema).expect("a command's results schema is valid JSON");
    let kind = root.pointer_mut(&format!(
        "/properties/{RESULTS_KEY}/items/properties/{KIND_KEY}"
    ))?;
    kind.as_object_mut()
        .expect("a `kind` property is an object schema")
        .insert(
            "enum".to_owned(),
            serde_json::Value::Array(
                kinds
                    .iter()
                    .map(|(label, _)| serde_json::Value::String(label.clone()))
                    .collect(),
            ),
        );
    Some(root.to_string())
}

pub(crate) fn results_schema(item: &str) -> String {
    let envelope = format!(
        r#"{{"type":"object","properties":{{"{RESULTS_KEY}":{{"type":"array","items":{item}}},"{NOTE_KEY}":{{"type":"string"}}}},"required":["{RESULTS_KEY}","{NOTE_KEY}"]}}"#
    );
    let mut root: serde_json::Value = serde_json::from_str(&envelope)
        .expect("a command's assembled results schema is valid JSON");
    close_objects(&mut root);
    root.to_string()
}

/// Recursively set `additionalProperties: false` on every object node in a JSON Schema — the
/// closed-object shape OpenAI/codex structured output requires (and claude accepts). One statement,
/// applied by [`results_schema`] to the whole envelope (and so to each command's embedded item).
fn close_objects(node: &mut serde_json::Value) {
    match node {
        serde_json::Value::Object(map) => {
            if map.get("type").and_then(serde_json::Value::as_str) == Some("object") {
                map.insert(
                    "additionalProperties".to_owned(),
                    serde_json::Value::Bool(false),
                );
            }
            for child in map.values_mut() {
                close_objects(child);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(close_objects),
        _ => {}
    }
}

/// Configuration shared by every synthesis-backed command.
/// One kind of a run's effective taxonomy, as recorded: its label plus the fingerprint of its
/// description — the (identity, content-hash) pattern of [`crate::rules::ActiveRule`] applied to
/// the vocabulary lever, so a finding attributes to the *version* of a kind that was in play and a
/// sharpened description splits from its predecessor's record rather than blending into it.
#[derive(Serialize)]
pub struct ActiveKind {
    pub kind: String,
    pub hash: String,
}

pub struct SynthOptions<'a> {
    /// The resolved model id for the run (the backend already applied `--model`, the shared default,
    /// or its own default), reported as the requested model until the response names what actually ran.
    pub model: &'a str,
    /// Number of synthesis runs to fan out concurrently; their results are unioned. 1 = single run.
    pub runs: usize,
    /// Hard per-run cost cap in dollars, passed to the CLI (each run of a fan-out carries its own).
    /// `None` = no cap.
    pub max_budget_usd: Option<f64>,
    /// Synthesis backend (`claude` | `codex`) — selects the CLI and shapes cost reporting (codex:
    /// tokens only); reported + recorded so the output says which backend ran.
    pub backend: &'a str,
    /// Codex reasoning effort, resolved + surfaced because it shapes cost; `None` for backends that
    /// don't use it (so it's never a hidden billed default).
    pub reasoning_effort: Option<&'a str>,
    /// Whether `--ranked` ordered the results (it shapes the prompt, so it's reported + recorded).
    pub ranked: bool,
    /// Whether `--kinds` classified the results (likewise prompt/schema-shaping, so reported + recorded).
    pub kinds: bool,
    /// Whether `--free-kinds` unlocked the taxonomy's enum on `kind` (deviation is deliberate and
    /// recorded, never ambient).
    pub free_kinds: bool,
    /// Arc-requested Claude tool grants (empty = none). Provider-owned tools are reported separately
    /// in the run boundary.
    pub allowed_tools: &'a [String],
    /// Repository root for the run — see [`crate::ai::Request`] for how it reaches allowed tools.
    pub dir: &'a Path,
    /// Human-readable descriptions of the context pieces included (for the run report).
    pub sources: &'a [String],
    /// The active rules (id + body fingerprint), recorded structured on the run — the exposure half
    /// of firing stats, version-aware.
    pub rules_active: &'a [crate::rules::ActiveRule],
    /// The effective taxonomy (kind + description fingerprint), recorded structured on the run —
    /// the same version-aware pattern applied to the vocabulary lever.
    pub taxonomy: &'a [ActiveKind],
    /// Notable context excluded by default (e.g. source files), surfaced so defaults aren't hidden.
    pub excluded: &'a [String],
    /// The active `.arc/settings.json` layers (user then project); empty = built-in defaults only.
    pub config: &'a [String],
    /// Command name (e.g. "suggest") — names the `--output` file and labels the doc's provenance.
    pub command: &'a str,
    /// Optional directory to also write the synthesis into, as `<command>.md`.
    pub output: Option<&'a Path>,
    /// Opt into target/user ambient customizations. The exact backend-specific additions and the
    /// residual provider-managed inputs are decomposed in the run boundary.
    pub ambient_memory: bool,
    /// JSON Schema for structured output (set whenever the verb declares a shape), or `None` for a
    /// prose verb's free-form narrative.
    pub schema: Option<&'a str>,
    /// When `Some(field)` (from `--fail-on-findings`), block — exit non-zero — if the structured
    /// output's `field` array is non-empty. `None` = no gating (default). Decoupled policy: the
    /// synthesis is unchanged; only the process exit code (and a loud line) reflect the gate.
    pub gate: Option<&'a str>,
    /// Preview the prompt + estimate without calling the model (zero spend).
    pub dry_run: bool,
    /// Emit machine-readable JSON instead of human text.
    pub json: bool,
    /// Append a record of this run to `~/.arc/logs/runs.jsonl` (real runs only; disable via settings).
    pub log: bool,
}

/// A file's text, optionally capped: the body, its original char count, and the cap it
/// was truncated to (if any) — so any truncation is reportable, never silent.
struct Capped {
    body: String,
    original_chars: usize,
    truncated_to: Option<usize>,
}

/// Read a file as text, optionally capped. `max` is an *optional, caller-chosen* cap (a compression
/// knob); by default (`None`) the whole file is read — context elision is never automatic. The
/// absent-vs-present-but-unreadable distinction comes from [`crate::read_optional`] (the one place
/// that classification lives): `Ok(None)` is absent, `Err` is unreadable.
fn read_file(path: &Path, max: Option<usize>) -> std::io::Result<Option<Capped>> {
    let Some(text) = crate::read_optional(path)? else {
        return Ok(None);
    };
    let original_chars = text.chars().count();
    let capped = match max {
        Some(cap) if original_chars > cap => Capped {
            body: format!(
                "{}\n…[truncated by arclite at {cap} chars]",
                text.chars().take(cap).collect::<String>()
            ),
            original_chars,
            truncated_to: Some(cap),
        },
        _ => Capped {
            body: text,
            original_chars,
            truncated_to: None,
        },
    };
    Ok(Some(capped))
}

/// A `sources` label for a file, making any caller-applied truncation explicit.
fn source_label(name: impl std::fmt::Display, cap: &Capped) -> String {
    match cap.truncated_to {
        Some(to) => format!("{name} ({} chars, truncated to {to})", cap.original_chars),
        None => format!("{name} ({} chars)", cap.original_chars),
    }
}

/// A `sources` label for a present-but-unreadable file — single-sourced so the wording can't drift.
fn unreadable_label(label: &str) -> String {
    format!("{label} (unreadable — skipped)")
}

/// A `sources` label for an explicit `--include` path that isn't there — distinct from
/// [`unreadable_label`] so a typo'd path reads as missing, not as present-but-unreadable.
fn missing_label(label: &str) -> String {
    format!("{label} (missing — skipped)")
}

/// Walk a directory gitignore-aware within `root` (the repo's ignore chain applies even below the
/// root — see [`crate::walk::configured`]), returning its files (sorted; `.git` skipped) and the
/// count of walk errors (unreadable entries) so callers can surface them, not drop them.
fn walk_files(root: &Path, dir: &Path) -> anyhow::Result<(Vec<PathBuf>, usize)> {
    let (entries, errors) = crate::walk::entries(root, dir)?;
    let mut files: Vec<PathBuf> = entries
        .into_iter()
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .map(ignore::DirEntry::into_path)
        .collect();
    files.sort();
    Ok((files, errors))
}

/// Expand each `--include` path (a file *or* a directory) into context text, applying the optional
/// caller cap and dropping any file an `--exclude` pattern matches. Skips any file already in context
/// — README/manifests (pre-seeded in `seen`) *and* any earlier `--include`/`--changed` file, recording
/// each one it adds — so overlapping inputs (an explicit file also under an included dir, or a
/// `--changed` file under one) aren't read or billed twice. Dirs are walked gitignore-aware.
fn gather_includes(
    root: &Path,
    paths: &[PathBuf],
    max: Option<usize>,
    excluder: &ignore::gitignore::Gitignore,
    seen: &mut SeenFiles,
    sources: &mut Vec<String>,
) -> anyhow::Result<String> {
    let mut ctx = String::new();
    let mut walked_dir = false;
    for path in paths {
        let is_dir = path.is_dir();
        walked_dir |= is_dir;
        let (files, walk_errors) = if is_dir {
            walk_files(root, path)?
        } else {
            (vec![path.clone()], 0)
        };
        let mut unreadable = 0usize;
        let mut duplicate = 0usize;
        let mut excluded = 0usize;
        // The largest excluded file, by on-disk size (the file is never read, so bytes, not chars).
        // A bare count can hide a whole source file inside an --exclude sweep; naming the biggest
        // drop keeps the coverage hole visible and the follow-up (--include it) derivable.
        let mut excluded_largest: Option<(String, u64)> = None;
        for file in &files {
            if excluder
                .matched_path_or_any_parents(file, false)
                .is_ignore()
            {
                excluded += 1;
                // An unprobeable size just isn't ranked — the count above still records the drop.
                if let Ok(meta) = file.metadata()
                    && excluded_largest
                        .as_ref()
                        .is_none_or(|(_, max)| meta.len() > *max)
                {
                    excluded_largest = Some((file.display().to_string(), meta.len()));
                }
                continue; // dropped by an --exclude pattern
            }
            let label = file.display().to_string();
            match add_unless_seen(file, &label, max, &mut ctx, sources, seen) {
                Added::Ok => {}
                Added::Duplicate => duplicate += 1, // already in context — not read or billed twice
                // An explicit --include that's absent reads as missing; one present-but-unreadable as
                // unreadable. Under a walked dir both fold into the unreadable tally (a file the walk
                // just listed then couldn't read is a mid-walk race, not a user's typo).
                Added::Missing if !is_dir => sources.push(missing_label(&label)),
                Added::Unreadable | Added::Missing => {
                    if is_dir {
                        unreadable += 1;
                    } else {
                        sources.push(unreadable_label(&label));
                    }
                }
            }
        }
        // Surface what was skipped under this path, rather than silently dropping or double-counting.
        if duplicate > 0 {
            sources.push(format!(
                "{duplicate} already-included file(s) under {} — skipped",
                path.display()
            ));
        }
        if excluded > 0 {
            // Disclose the largest drop beside the count (see excluded_largest above); when no
            // excluded file's size was probeable, the count stands alone rather than guessing.
            let largest = excluded_largest
                .take()
                .map(|(label, bytes)| format!(" (largest: {label}, {bytes} bytes)"))
                .unwrap_or_default();
            sources.push(format!(
                "{excluded} file(s) under {} excluded by --exclude — skipped{largest}",
                path.display()
            ));
        }
        if unreadable > 0 {
            sources.push(format!(
                "{unreadable} unreadable file(s) under {} — skipped",
                path.display()
            ));
        }
        if walk_errors > 0 {
            sources.push(format!(
                "{walk_errors} unwalkable entr(ies) under {} — skipped",
                path.display()
            ));
        }
    }
    // A walked --include directory yields the gitignore-filtered view, like the scan; surface it.
    if walked_dir {
        sources.push(format!("walked directories: {}", crate::walk::SCOPE_NOTE));
    }
    Ok(ctx)
}

/// Render the rules from `rule_sources` as a context block, recording which rule ids were included —
/// and which the settings' disabled list filtered out, so a shrunken ruleset is always disclosed.
fn gather_rules(
    rule_sources: &[PathBuf],
    disabled_rules: &[String],
    sources: &mut Vec<String>,
) -> anyhow::Result<(String, Vec<crate::rules::ActiveRule>)> {
    if rule_sources.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let (loaded, skipped, overridden) = crate::rules::load_sources(rule_sources)?;
    for src in &skipped {
        // A configured source that resolved to nothing (typo'd path, absent dir, or a non-`.md`
        // file): surface it in the manifest so a shrunken ruleset never goes unnoticed.
        sources.push(format!(
            "rules: source skipped — not a directory or .md file: {}",
            src.display()
        ));
    }
    for o in &overridden {
        // Later-source-wins is the designed override; each collision it resolved is disclosed so a
        // rule body silently dropping out of the active set can't go unnoticed.
        sources.push(format!(
            "rules: `{}` from {} overridden by {}",
            o.id,
            o.replaced.display(),
            o.winner.display()
        ));
    }
    let (rules, disabled) = crate::rules::partition_disabled(loaded, disabled_rules);
    if !disabled.is_empty() {
        sources.push(format!(
            "rules disabled in settings ({}): {}",
            disabled.len(),
            disabled
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if rules.is_empty() {
        // Distinguish an empty ruleset from one emptied by the disabled list — different remedies.
        if disabled.is_empty() {
            sources.push(format!(
                "rules: none found in {} source(s)",
                rule_sources.len()
            ));
        } else {
            sources.push("rules: every resolved rule is disabled in settings".to_owned());
        }
        return Ok((String::new(), Vec::new()));
    }
    let active: Vec<crate::rules::ActiveRule> = rules
        .iter()
        .map(|r| crate::rules::ActiveRule {
            id: r.id.clone(),
            hash: crate::rules::fingerprint(&r.body),
        })
        .collect();
    sources.push(format!(
        "rules ({}): {}",
        rules.len(),
        active
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // The weigh-against-these framing travels with the block itself — stated once, present exactly
    // when rules are — rather than each command's prompt referencing rules that may not be in context.
    Ok((
        format!(
            "\nRules (the standards to weigh the repository against):\n{}\n",
            crate::rules::render(&rules)
        ),
        active,
    ))
}

/// Render the repo's open findings ledger (`.arc/findings/open/*.md`) as a context block. With
/// `recheck` (the verify verb) the findings are presented for re-checking against the current code;
/// otherwise a run is told to hunt *beyond* them rather than re-surface them. Absent/empty ledger → no
/// block. A finding is Markdown like a rule, so the rule loader/renderer applies — each rendered under a
/// `## <id>` heading, so a verdict can key back to it — guarded against an absent dir.
/// The tracked items — the open agenda — rendered as the align verb's subject: the intended order
/// first (`order.json`, its integrity computed deterministically — duplicates, ids resolving to no
/// item, open items it omits — each disclosed, so order drift is a flagged state, never silent
/// rot), then each open item under its id (`.md` files, stem = id, loaded the same way rules are).
/// The items are material to judge, never instructions to follow; an absent or empty agenda is
/// stated and yields an empty block — a judged-empty subject, not an error. A *malformed* order
/// file is a hard error, like any explicitly-present structured source.
fn gather_agenda(root: &Path, sources: &mut Vec<String>) -> anyhow::Result<String> {
    let items = load_ledger_dir(&crate::items_open_dir(root), "items")?;
    let order_path = crate::items_order_path(root);
    let order: Option<Vec<String>> = match crate::read_optional(&order_path)
        .with_context(|| format!("cannot read {}", order_path.display()))?
    {
        Some(text) => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Order {
                order: Vec<String>,
            }
            let parsed: Order = serde_json::from_str(&text)
                .with_context(|| format!("invalid order file {}", order_path.display()))?;
            Some(parsed.order)
        }
        None => None,
    };
    if items.is_empty() && order.is_none() {
        sources.push("items: none (.arc/items/open absent or empty)".to_owned());
        return Ok(String::new());
    }
    // Order integrity, computed once here so the run's context, its sources line, and any later
    // consumer all see the same verdicts.
    let ids: std::collections::BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let mut listed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut dangling: Vec<&str> = Vec::new();
    let mut duplicated: Vec<&str> = Vec::new();
    for id in order.iter().flatten() {
        if !listed.insert(id.as_str()) {
            duplicated.push(id);
        }
        if !ids.contains(id.as_str()) {
            dangling.push(id);
        }
    }
    let unlisted: Vec<&str> = items
        .iter()
        .map(|i| i.id.as_str())
        .filter(|id| !listed.contains(id))
        .collect();
    let integrity = match &order {
        None => "no order file".to_owned(),
        Some(_) if unlisted.is_empty() && dangling.is_empty() && duplicated.is_empty() => {
            "order: complete".to_owned()
        }
        Some(_) => {
            let mut parts = Vec::new();
            if !unlisted.is_empty() {
                parts.push(format!("{} open item(s) unlisted", unlisted.len()));
            }
            if !dangling.is_empty() {
                parts.push(format!("{} id(s) resolve to no item", dangling.len()));
            }
            if !duplicated.is_empty() {
                parts.push(format!("{} duplicated id(s)", duplicated.len()));
            }
            format!("order: {}", parts.join(", "))
        }
    };
    sources.push(format!(
        "items ({}): {} · {}",
        items.len(),
        items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        integrity
    ));
    let mut block = String::from("\nTracked items (the open agenda — the subject to audit):\n");
    match &order {
        Some(list) => {
            block.push_str("\nIntended order (order.json):\n");
            for (i, id) in list.iter().enumerate() {
                block.push_str(&format!("{}. {id}\n", i + 1));
            }
            if !(unlisted.is_empty() && dangling.is_empty() && duplicated.is_empty()) {
                block.push_str(&format!(
                    "Order integrity: unlisted [{}] · dangling [{}] · duplicated [{}]\n",
                    unlisted.join(", "),
                    dangling.join(", "),
                    duplicated.join(", ")
                ));
            }
        }
        None => block.push_str("\nNo order file (.arc/items/order.json is absent).\n"),
    }
    for item in &items {
        block.push_str(&format!("\n## {}\n{}\n", item.id, item.body));
    }
    Ok(block)
}

/// Load a `.md` ledger directory (the findings ledger, the items agenda): absent = a legitimate
/// empty ledger; a *present non-directory* squatting on the path = damage, reported as unreadable
/// rather than read as empty — a run must not quietly proceed as though the repo recorded nothing.
/// One home for the absence/damage distinction, so the ledgers can't drift in how they draw it.
fn load_ledger_dir(dir: &Path, what: &str) -> anyhow::Result<Vec<crate::rules::Rule>> {
    let access = |e: std::io::Error| anyhow::anyhow!("cannot access {}: {e}", dir.display());
    if crate::try_is_dir(dir).map_err(access)? {
        crate::rules::load(dir)
    } else {
        anyhow::ensure!(
            !dir.try_exists().map_err(access)?,
            "{} exists but is not a directory — the {what} ledger is unreadable, not empty",
            dir.display()
        );
        Ok(Vec::new())
    }
}

fn gather_findings(
    root: &Path,
    sources: &mut Vec<String>,
    recheck: bool,
) -> anyhow::Result<String> {
    let entries = load_ledger_dir(&crate::findings_open_dir(root), "findings")?;
    if entries.is_empty() {
        return Ok(String::new());
    }
    sources.push(format!("findings ledger ({} open)", entries.len()));
    let framing = if recheck {
        "Open findings recorded in this repo's ledger, each under a `## <id>` heading — re-check each against the current code:"
    } else {
        "Findings already recorded in this repo's ledger (surface NEW issues beyond these; do not re-report them):"
    };
    Ok(format!("\n{framing}\n{}\n", crate::rules::render(&entries)))
}

/// Render prior runs' stored structured results (`--from`, the aggregate verb) as a context block:
/// each run under its id with the command + repo it examined, its items rendered one by one — the
/// raw material of a cross-run merge. Every named run must exist and carry structured results;
/// anything less is an error naming the id, because an aggregate silently missing a source would
/// judge a different question than the one asked.
fn gather_runs(ids: &[String], sources: &mut Vec<String>) -> anyhow::Result<String> {
    let mut text = String::from("\nResults of the prior runs to aggregate:\n");
    for id in ids {
        let id = crate::commands::log::resolve_id(id)?;
        let stored = crate::commands::log::load_stored(&id)?.ok_or_else(|| {
            anyhow::anyhow!(
                "no stored result for run `{id}` — aggregate reads each named run's stored results \
                 (runs predating the store, or made with logging off, aren't kept)"
            )
        })?;
        let run = crate::commands::log::stored_run(&stored);
        let command = &crate::log::field(&run, "command");
        let repo = crate::log::field(&run, "repo");
        let items = stored
            .get("structured")
            .and_then(|s| s.get(RESULTS_KEY))
            .and_then(|r| r.as_array())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "run `{id}` has no structured results to aggregate (`arc log {id}` shows what it holds)"
                )
            })?;
        // The invocation spelling comes from the one builder every surface shares — never a
        // hardcoded `arc run` literal a binary/run-group rename would strand.
        text.push_str(&format!(
            "\n## run {id} — `{}` on {repo} ({} result(s))\n",
            crate::commands::promote::run_invocation(command),
            items.len()
        ));
        for item in items {
            text.push_str(&format!("\n{}\n", item_bullets(item)));
        }
        sources.push(format!(
            "run {id} (`{command}` on {repo}, {} result(s))",
            items.len()
        ));
    }
    Ok(text)
}

/// Read `path` (capped at `max`) and, on success, append its body to `text` under `label` and record
/// the source label. `Ok(true)` = read and appended, `Ok(false)` = absent, `Err` = present but
/// unreadable (the [`read_file`]/[`crate::read_optional`] distinction); the caller surfaces a miss
/// however it needs.
fn append_file(
    label: &str,
    path: &Path,
    max: Option<usize>,
    text: &mut String,
    sources: &mut Vec<String>,
) -> std::io::Result<bool> {
    let Some(cap) = read_file(path, max)? else {
        return Ok(false);
    };
    sources.push(source_label(label, &cap));
    text.push_str(&format!("\n{label}:\n{}\n", cap.body));
    Ok(true)
}

/// The canonical path of `path`, if it resolves — the identity recorded in (and checked against)
/// `seen`, so README/manifests and every `--include`/`--changed` file dedupe by identity, not by
/// how the path was spelled.
fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// What [`add_unless_seen`] did with a file.
enum Added {
    /// Appended and recorded in `seen`.
    Ok,
    /// Already in `seen` (a README/manifest or an earlier include) — not re-added.
    Duplicate,
    /// Present but unreadable.
    Unreadable,
    /// Absent.
    Missing,
}

/// A successfully-read repository-content file. Keep the original path spelling alongside the
/// canonical dedup identity: Git classifies the path the user/repo named, not the symlink target
/// canonicalization may lead to.
struct IncludedFile {
    path: PathBuf,
    label: String,
    canonical: Option<PathBuf>,
}

/// Both views of the files already placed in context: canonical identities prevent duplicate
/// content, while the successfully-read paths feed the Git-state inventory once gathering is done.
#[derive(Default)]
struct SeenFiles {
    identities: Vec<PathBuf>,
    included: Vec<IncludedFile>,
}

/// Add `path` to the context unless it's already there: skip if its canonical path is in `seen`,
/// else append it (as `label`, capped at `max`) and record it in `seen`. The single place a context
/// file is appended and deduped — README/manifests and every `--include`/`--changed` file share it,
/// so the dedup identity can't drift. Callers handle the outcome (disclose, count, or ignore) as fits.
fn add_unless_seen(
    path: &Path,
    label: &str,
    max: Option<usize>,
    text: &mut String,
    sources: &mut Vec<String>,
    seen: &mut SeenFiles,
) -> Added {
    let canon = canonical(path);
    if let Some(c) = &canon
        && seen.identities.contains(c)
    {
        return Added::Duplicate;
    }
    match append_file(label, path, max, text, sources) {
        Ok(true) => {
            if let Some(c) = &canon {
                seen.identities.push(c.clone());
            }
            seen.included.push(IncludedFile {
                path: path.to_path_buf(),
                label: label.to_owned(),
                canonical: canon,
            });
            Added::Ok
        }
        Ok(false) => Added::Missing,
        Err(_) => Added::Unreadable,
    }
}

/// Append an auto-included file (README/manifest): a present-but-unreadable one is surfaced, an
/// absent one stays silent (these are optional, so their absence isn't worth noting).
fn add_file(
    path: &Path,
    label: &str,
    max: Option<usize>,
    text: &mut String,
    sources: &mut Vec<String>,
    seen: &mut SeenFiles,
) {
    if let Added::Unreadable = add_unless_seen(path, label, max, text, sources, seen) {
        sources.push(unreadable_label(label));
    }
}

/// The non-inference rule carried inside every Git-state block. The motivating failure was exactly
/// this category error: a tracked project file mentioned an ignored signing key, and the mention was
/// mistaken for evidence that the key itself was committed.
const GIT_REFERENCE_RULE: &str = "Only paths explicitly named by this block have known Git state. A path merely mentioned inside included content has unknown Git state; never infer that a tracked referencing file makes its target tracked or committed.";

#[derive(Default)]
struct GitPathSet {
    paths: BTreeSet<String>,
    undecodable: usize,
}

struct GitSnapshot {
    tracked: GitPathSet,
    dirty: GitPathSet,
    untracked: GitPathSet,
    ignored: GitPathSet,
}

struct GitTruthContext {
    text: String,
    source: String,
}

/// Run Git in the target directory with a pinned message locale. Paths are consumed only from `-z`
/// stdout below; stderr is human diagnostic text and is never interpreted except for the shared
/// not-a-repository distinction on the initial probe.
fn git_output(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    ai::command("git")
        .map_err(|e| format!("could not prepare git: {e:#}"))?
        .env("LC_ALL", "C")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = git_output(root, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`git {}` exited {}: {}",
            args.join(" "),
            output.status,
            stderr.trim()
        ));
    }
    Ok(output.stdout)
}

/// Parse Git's NUL-delimited path output without lossy decoding. A path that cannot be represented
/// in the JSON prompt is omitted and counted; turning it into replacement characters would name a
/// different path and manufacture false evidence.
fn git_path_set(bytes: &[u8]) -> GitPathSet {
    let mut set = GitPathSet::default();
    for raw in bytes
        .split(|&byte| byte == 0)
        .filter(|path| !path.is_empty())
    {
        match std::str::from_utf8(raw) {
            Ok(path) => {
                // When the target itself is an outer worktree's untracked directory, Git collapses
                // the whole requested scope to `./`. Normalize that sentinel for JSON and matching.
                set.paths.insert(if path == "./" {
                    ".".to_owned()
                } else {
                    path.to_owned()
                });
            }
            Err(_) => set.undecodable += 1,
        }
    }
    set
}

fn query_git_paths(root: &Path, args: &[&str]) -> Result<GitPathSet, String> {
    git_stdout(root, args).map(|stdout| git_path_set(&stdout))
}

/// Collect the target-relative index and exception sets. `ls-files` emits paths relative to the
/// `-C` directory, and `git diff --relative` does the same, avoiding any need to parse/strip a
/// newline-delimited worktree prefix. Directory entries in the untracked/ignored sets stay collapsed.
fn git_snapshot(root: &Path) -> Result<GitSnapshot, String> {
    let probe = git_output(root, &["rev-parse", "--is-inside-work-tree"])?;
    if !probe.status.success() {
        let stderr = String::from_utf8_lossy(&probe.stderr);
        if probe.status.code() == Some(128) && crate::git_stderr_says_not_a_repo(&stderr) {
            return Err("target is not a Git work tree".to_owned());
        }
        return Err(format!(
            "Git work-tree probe failed ({}): {}",
            probe.status,
            stderr.trim()
        ));
    }
    if probe.stdout.as_slice().trim_ascii() != b"true" {
        return Err("target is not a Git work tree".to_owned());
    }

    // A target directory ignored by an ancestor worktree makes collapsed `ls-files --ignored`
    // fail in Git itself. Probe the scope root directly and represent it with `.`; exact tracked
    // entries still win during classification (a force-added file can live under an ignored dir).
    let root_ignore = git_output(root, &["check-ignore", "--quiet", "--no-index", "--", "."])?;
    let root_ignored = match root_ignore.status.code() {
        Some(0) => true,
        Some(1) => false,
        _ => {
            return Err(format!(
                "`git check-ignore` exited {}: {}",
                root_ignore.status,
                String::from_utf8_lossy(&root_ignore.stderr).trim()
            ));
        }
    };

    let tracked = query_git_paths(root, &["ls-files", "--cached", "-z", "--", "."])?;
    let mut dirty = query_git_paths(
        root,
        &["ls-files", "--modified", "--deleted", "-z", "--", "."],
    )?;
    let staged = query_git_paths(
        root,
        &[
            "diff",
            "--cached",
            "--name-only",
            "--relative",
            "--no-renames",
            "-z",
            "--",
            ".",
        ],
    )?;
    dirty.paths.extend(staged.paths);
    dirty.undecodable += staged.undecodable;
    let untracked = query_git_paths(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
            "--",
            ".",
        ],
    )?;
    let ignored = if root_ignored {
        GitPathSet {
            paths: BTreeSet::from([".".to_owned()]),
            undecodable: 0,
        }
    } else {
        query_git_paths(
            root,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "--directory",
                "--no-empty-directory",
                "-z",
                "--",
                ".",
            ],
        )?
    };
    Ok(GitSnapshot {
        tracked,
        dirty,
        untracked,
        ignored,
    })
}

/// Translate a successfully-read path into Git's target-relative spelling. A lexical escape is
/// outside scope; a non-UTF-8 spelling is unknown because the JSON channel cannot name it exactly.
fn git_relative_path(root: &Path, path: &Path) -> Result<String, &'static str> {
    let relative = path.strip_prefix(root).map_err(|_| "outside_target")?;
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("outside_target");
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("outside_target");
    }
    normalized
        .to_str()
        // Git prints `/` separators even on Windows; on Unix this is a no-op and leaves literal
        // backslashes untouched.
        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
        .ok_or("unknown")
}

/// `ls-files --directory` collapses an untracked/ignored directory to `dir/`; a context file below
/// it inherits that state even though its full path is intentionally absent from the compact set.
fn path_or_collapsed_parent_is_listed(paths: &BTreeSet<String>, path: &str) -> bool {
    paths.contains(".")
        || paths.contains(path)
        || paths
            .iter()
            .any(|candidate| candidate.ends_with('/') && path.starts_with(candidate))
}

/// Whether this exact spelling (or a non-root collapsed parent) has a specific Git fact. The `.`
/// scope sentinel is deliberately excluded: a canonical spelling that matches a tracked entry must
/// beat the blanket ignored/untracked state of its target directory.
fn path_has_specific_state(snapshot: &GitSnapshot, path: &str) -> bool {
    snapshot.tracked.paths.contains(path)
        || snapshot.ignored.paths.contains(path)
        || snapshot.untracked.paths.contains(path)
        || snapshot
            .ignored
            .paths
            .iter()
            .chain(&snapshot.untracked.paths)
            .any(|candidate| {
                candidate != "." && candidate.ends_with('/') && path.starts_with(candidate)
            })
}

fn git_state_for_path(snapshot: &GitSnapshot, path: &str) -> &'static str {
    if snapshot.tracked.paths.contains(path) {
        if snapshot.dirty.paths.contains(path) {
            "index_tracked_dirty"
        } else {
            "index_tracked_clean"
        }
    } else if path_or_collapsed_parent_is_listed(&snapshot.ignored.paths, path) {
        "ignored_untracked"
    } else if path_or_collapsed_parent_is_listed(&snapshot.untracked.paths, path) {
        "untracked"
    } else {
        // A submodule/nested-worktree descendant and an entry omitted from a partial query both land
        // here. Absence is not evidence of ordinary untracked state.
        "unknown"
    }
}

fn unknown_git_rows(root: &Path, included: &[IncludedFile]) -> Vec<serde_json::Value> {
    included
        .iter()
        .map(|file| match git_relative_path(root, &file.path) {
            Ok(path) => serde_json::json!({"path": path, "git_state": "unknown"}),
            Err(state) => serde_json::json!({"path": file.label, "git_state": state}),
        })
        .collect()
}

fn render_git_truth(value: &serde_json::Value, source: String) -> GitTruthContext {
    GitTruthContext {
        text: format!(
            "\nVersion-control truth (JSON; path names and metadata only):\n{}\n",
            serde_json::to_string_pretty(value)
                .expect("the in-memory Git truth value always serializes")
        ),
        source,
    }
}

/// Build the default-on, deterministic Git grounding that constrains every synthesis verb. The
/// exception lists close the reference gap: ignored/untracked paths can be named even though their
/// contents were correctly absent from the ordinary ignore-aware context walk.
fn gather_git_truth(root: &Path, included: &[IncludedFile]) -> GitTruthContext {
    let snapshot = match git_snapshot(root) {
        Ok(snapshot) => snapshot,
        Err(reason) => {
            let value = serde_json::json!({
                "available": false,
                "reason": reason,
                "included_repository_files": unknown_git_rows(root, included),
                "claim_rule": GIT_REFERENCE_RULE,
            });
            return render_git_truth(
                &value,
                "git state: unavailable (make no tracking or commit claims)".to_owned(),
            );
        }
    };

    let canonical_root = canonical(root);
    let rows: Vec<serde_json::Value> = included
        .iter()
        .map(|file| {
            let primary = git_relative_path(root, &file.path);
            let canonical = canonical_root
                .as_deref()
                .zip(file.canonical.as_deref())
                .map(|(canonical_root, path)| git_relative_path(canonical_root, path));
            // `a/../b` cannot be collapsed lexically when `a` may be a symlink: the filesystem first
            // follows `a`, so the actual file can be outside the target. Canonical identity must drive
            // any path containing `..`; if it cannot be established, its Git state remains unknown.
            let has_parent = file
                .path
                .strip_prefix(root)
                .is_ok_and(|path| path.components().any(|c| c == Component::ParentDir));
            if has_parent {
                return match canonical.unwrap_or(Err("unknown")) {
                    Ok(path) => serde_json::json!({
                        "git_state": git_state_for_path(&snapshot, &path),
                        "path": path,
                    }),
                    Err(state) => serde_json::json!({"path": file.label, "git_state": state}),
                };
            }
            match primary {
                Ok(path) => {
                    // Preserve the supplied spelling when Git knows it (notably symlinks). A
                    // canonical fallback recovers harmless aliases such as case-only spellings on
                    // case-insensitive filesystems without overriding a real Git entry.
                    let path = if path_has_specific_state(&snapshot, &path) {
                        path
                    } else {
                        canonical
                            .and_then(Result::ok)
                            .filter(|path| path_has_specific_state(&snapshot, path))
                            .unwrap_or(path)
                    };
                    let state = git_state_for_path(&snapshot, &path);
                    serde_json::json!({"path": path, "git_state": state})
                }
                Err(state) => serde_json::json!({"path": file.label, "git_state": state}),
            }
        })
        .collect();

    let omitted = snapshot.tracked.undecodable
        + snapshot.dirty.undecodable
        + snapshot.untracked.undecodable
        + snapshot.ignored.undecodable;
    let dirty_count = snapshot.dirty.paths.len();
    let untracked_count = snapshot.untracked.paths.len();
    let ignored_count = snapshot.ignored.paths.len();
    let value = serde_json::json!({
        "available": true,
        "scope": "Git-listed paths are target-relative; an included file outside the target keeps its supplied path and is labeled outside_target",
        "included_repository_files": rows,
        "changed_paths": snapshot.dirty.paths,
        "changed_path_semantics": "A changed path may be modified, added, deleted, or the old side of a rename; its presence here alone does not prove the path currently exists or is index-tracked.",
        "untracked_present": snapshot.untracked.paths,
        "ignored_untracked_present": snapshot.ignored.paths,
        "undecodable_path_records_omitted": omitted,
        "semantics": {
            "index_tracked_clean": "present in Git's index with no staged or unstaged change reported; this alone does not prove the displayed bytes exist in HEAD",
            "index_tracked_dirty": "present in Git's index and changed; the displayed bytes are not proven committed",
            "untracked": "reported present and untracked by Git",
            "ignored_untracked": "reported present, untracked, and ignored by Git",
            "unknown": "Git state was not established; do not infer it",
            "outside_target": "outside this Git snapshot's target scope",
        },
        "claim_rule": GIT_REFERENCE_RULE,
        "content_disclosure": "The snapshot adds path/index metadata only; it does not add the contents of listed untracked or ignored paths.",
    });
    render_git_truth(
        &value,
        format!(
            "git state: path names only ({} repository file(s) classified; {dirty_count} changed path(s); {untracked_count} untracked; {ignored_count} ignored; {omitted} undecodable record(s) omitted; snapshot adds no file contents)",
            included.len()
        ),
    )
}

/// Assembled repo context, a record of every source and what was excluded, and the repo
/// root (granted to tools when any are allowed).
pub struct Context {
    pub text: String,
    pub sources: Vec<String>,
    pub excluded: Vec<String>,
    pub root: PathBuf,
    /// The active rules in the context (after the disabled-list filter) as (id, body-fingerprint)
    /// pairs — recorded structured on the run so firing stats can join findings to the exposed
    /// rule *version*, never by reparsing display prose.
    pub rules_active: Vec<crate::rules::ActiveRule>,
}

/// Files with uncommitted changes (staged, unstaged, or untracked) under `root`, per git —
/// backing `--changed`. Returns `(paths, deleted, undecodable)`: the readable changed files, the
/// deletion count, and how many changed paths weren't valid UTF-8 (skipped, for the caller to
/// disclose — a lossily-decoded name would point the later read at a mangled path). `Err(reason)`
/// means git itself couldn't be consulted — kept distinct so a failed scope never masquerades
/// as a clean "no changes" result.
fn changed_files(root: &Path) -> Result<(Vec<PathBuf>, usize, usize), String> {
    // -z: NUL-terminated records with paths emitted verbatim — no C-style quoting/escaping — so a
    // filename with spaces or non-ASCII (valid UTF-8) bytes survives the parse, where plain
    // `--porcelain` would C-quote it (e.g. `"caf\303\251.rs"`) and a literal parse would drop it.
    let output = git_output(root, &["status", "--porcelain", "-z"])?;
    if !output.status.success() {
        return Err(format!(
            "git exited with {} (is {} a git repository?)",
            output.status,
            root.display()
        ));
    }
    // Each record is two status chars and a space, then the path. A rename/copy (status 'R'/'C') is
    // followed by a second record holding its original path — under -z the new path comes first and the
    // ` -> ` separator is dropped — so that trailing field carries no status prefix: consume and discard
    // it rather than mis-reading its bytes as another changed path. Records are split as *bytes*
    // (git emits raw path bytes): each path must then decode as UTF-8 exactly, because it's rejoined
    // to the root and read — a lossy decode would mangle the name and read the wrong path. An
    // undecodable path is skipped and counted, for the caller to disclose.
    const PORCELAIN_PATH_OFFSET: usize = 3;
    let mut records = output.stdout.split(|&b| b == 0);
    let mut changed = Vec::new();
    let mut deleted = 0usize;
    let mut undecodable = 0usize;
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue; // -z output ends in a NUL, so the final split is empty
        }
        let status = record;
        // A deletion ('D' in the index or worktree status column) has no content to feed context, so
        // exclude and count it — disclosed by the caller as a deletion rather than later surfacing as
        // a "missing — skipped" path indistinguishable from a typo'd `--include`.
        if matches!(status.first(), Some(b'D')) || matches!(status.get(1), Some(b'D')) {
            deleted += 1;
        } else if let Some(path) = record
            .get(PORCELAIN_PATH_OFFSET..)
            .filter(|p| !p.is_empty())
        {
            match std::str::from_utf8(path) {
                Ok(path) => changed.push(root.join(path)),
                Err(_) => undecodable += 1,
            }
        }
        if matches!(status.first(), Some(b'R' | b'C')) {
            records.next(); // a rename/copy carries its original path in the next record — discard it
        }
    }
    Ok((changed, deleted, undecodable))
}

/// The target repo's commit state at run time — `HEAD`'s short sha, suffixed `-dirty` when the
/// worktree differs from it — anchoring the run record (and any findings promoted from it) to the
/// code state the run actually judged. `None` when there is legitimately no commit to anchor (not a
/// git repo, or an unborn HEAD): a run on a plain directory is fine, so that absence stays silent.
/// A probe that *breaks* — git missing, a spawn failure, a status probe failing inside a known repo —
/// is unreadable, not absent: the anchor is still dropped, but with a warning, so a broken provenance
/// check can't masquerade as "no commit".
fn repo_commit(root: &Path) -> Option<String> {
    let unreadable = |what: &str| {
        eprintln!("arclite: run not commit-anchored — {what}");
    };
    // `--verify --quiet` gives exit codes that *distinguish* the benign absences from breakage —
    // machine-readable semantics, not stderr prose: 0 = HEAD resolves; 1 = verification failed
    // quietly (an unborn HEAD — a repo with no commits yet, legitimately nothing to anchor);
    // 128 = fatal (not a repository — benign here — or a corrupt one, whose stderr says which).
    let head = match git_output(
        root,
        &["rev-parse", "--verify", "--quiet", "--short", "HEAD"],
    ) {
        Ok(out) => out,
        Err(e) => {
            unreadable(&format!("git couldn't run ({e})"));
            return None;
        }
    };
    if !head.status.success() {
        // Exit 1 (quiet verification failure) = unborn HEAD: silently un-anchored. Exit 128 with
        // git's not-a-repository wording (the single-sourced match — `git_output` pins LC_ALL=C,
        // so the localized stderr reads the same words everywhere) = a plain directory: also
        // silently un-anchored. Anything else — repository corruption, a
        // locked object store — is unreadable, not absent, and warns before the anchor is dropped.
        let stderr = String::from_utf8_lossy(&head.stderr);
        let benign = head.status.code() == Some(1)
            || (head.status.code() == Some(128) && crate::git_stderr_says_not_a_repo(&stderr));
        if !benign {
            unreadable(&format!(
                "git rev-parse failed in a way that isn't \"not a repo\" ({})",
                stderr.trim()
            ));
        }
        return None;
    }
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    if sha.is_empty() {
        return None;
    }
    // Dirty = any uncommitted change (staged, unstaged, or untracked): the judged code went beyond
    // HEAD, and a finding anchored to the bare sha would overclaim. HEAD resolved, so this *is* a git
    // repo — a status probe failing now is unreadable (warned above the None), never silently absent,
    // and never presented as a clean commit.
    let status = match git_output(root, &["status", "--porcelain"]) {
        Ok(out) if out.status.success() => out,
        Ok(out) => {
            unreadable(&format!(
                "git status failed in a repo whose HEAD resolved ({})",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            return None;
        }
        Err(e) => {
            unreadable(&format!("git status couldn't run ({e})"));
            return None;
        }
    };
    if status.stdout.is_empty() {
        Some(sha)
    } else {
        Some(format!("{sha}-dirty"))
    }
}

/// What a run gathers into context — the shaping levers, grouped so adding one is a field, not a new
/// positional argument. Built from the run's args at the call site (the CLI↔synth marshal stays there).
pub struct ContextSpec<'a> {
    pub includes: &'a [PathBuf],
    pub rule_sources: &'a [PathBuf],
    /// Rule ids the settings disable — filtered out of the context, with the filtering disclosed.
    pub disabled_rules: &'a [String],
    pub max: Option<usize>,
    pub changed: bool,
    pub exclude: &'a [String],
    pub scan: bool,
    pub findings: bool,
    /// Auto-load the open findings ledger framed for *re-checking* (the verify verb), distinct from
    /// `findings`, which loads it framed for hunting *beyond* what's already known.
    pub recheck_findings: bool,
    /// Auto-load the tracked items — the open agenda and its spine — as the run's subject (the
    /// align verb).
    pub agenda: bool,
    /// Logged run ids (`--from`, the aggregate verb) whose stored structured results become context —
    /// the material a cross-run merge judges. Empty for every other verb.
    pub from_runs: &'a [String],
}

/// Assemble the repo context shared by every synthesis command: unless `scan` is false, the scan
/// summary + the manifests an inspect walk detects; the README; any `--include`d files/dirs (and, with
/// `changed`, git-changed files); deterministic Git state; rules; and the open ledger (with `findings`,
/// framed to hunt past it; with `recheck_findings`, framed to re-check it) — tracking each source (and
/// what's excluded) for the run report. `max` is the optional caller cap; by default files are read
/// whole. The prompt differs per command.
pub fn gather_context(path: &Path, spec: &ContextSpec) -> anyhow::Result<Context> {
    let &ContextSpec {
        includes,
        rule_sources,
        disabled_rules,
        max,
        changed,
        exclude,
        scan,
        findings,
        recheck_findings,
        agenda,
        from_runs,
    } = spec;
    // The repo scan (an inspect walk) yields the scan summary and the manifests it detects. `--no-scan`
    // (scan=false) drops both — and the content walk itself; the root still resolves directly for
    // --include/--changed, the README, and the target-wide Git path-state snapshot.
    let (report, root) = if scan {
        let (report, root) = crate::commands::inspect::gather(path)?;
        (Some(report), root)
    } else {
        // No scan to validate the path, so check it here — an unreadable target surfaces, a missing or
        // non-directory one says so (the same distinction inspect::gather makes).
        let root = crate::commands::resolve_root(path)?;
        anyhow::ensure!(
            crate::try_is_dir(&root)
                .map_err(|e| anyhow::anyhow!("cannot access {}: {e}", root.display()))?,
            "{} is not an existing directory",
            root.display()
        );
        (None, root)
    };

    let mut text = String::new();
    let mut sources: Vec<String> = Vec::new();
    let mut seen = SeenFiles::default();

    if let Some(report) = &report {
        text = format!(
            "Repository scan (JSON):\n{}\n",
            serde_json::to_string_pretty(report)?
        );
        sources.push("repository scan".to_owned());
    }

    add_file(
        &root.join("README.md"),
        "README.md",
        max,
        &mut text,
        &mut sources,
        &mut seen,
    );
    // The manifests come from the scan (root *or* nested) — included only when scanning, keeping the
    // scan's findings and the context consistent and making a default run substantive on repos whose
    // manifests live in subprojects, not at the root.
    if let Some(report) = &report {
        for rel in &report.manifest_paths {
            add_file(
                &root.join(rel),
                rel,
                max,
                &mut text,
                &mut sources,
                &mut seen,
            );
        }
    }
    // Resolve --include paths against the target repo (not arclite's cwd); `~/` and absolute paths
    // stand on their own — the shared crate::resolve_path rule, same as ruleset sources.
    let mut includes: Vec<PathBuf> = includes
        .iter()
        .map(|p| crate::resolve_path(&root, p))
        .collect();
    // --changed: scope to git-changed files — same group as --include, not special to any command.
    // A git failure aborts loudly rather than silently passing as a clean tree.
    if changed {
        let (files, deleted, undecodable) = changed_files(&root)
            .map_err(|reason| anyhow::anyhow!("--changed could not consult git: {reason}"))?;
        // Every skipped class is disclosed beside the count it was excluded from — deletions have
        // no content to read; a non-UTF-8 path can't be named without mangling it.
        let mut notes = Vec::new();
        if deleted > 0 {
            notes.push(format!("{deleted} deleted, skipped"));
        }
        if undecodable > 0 {
            notes.push(format!("{undecodable} non-UTF-8 path(s), skipped"));
        }
        let suffix = if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join("; "))
        };
        sources.push(match (files.is_empty(), notes.is_empty()) {
            (true, true) => "changed: no git changes found".to_owned(),
            (true, false) => format!("changed: nothing readable to include{suffix}"),
            (false, _) => format!("changed: {} git-changed file(s){suffix}", files.len()),
        });
        includes.extend(files);
    }
    // Compile the --exclude patterns (gitignore-style) once, applied to the walked include/changed
    // files in gather_includes. No patterns → matches nothing (no filtering).
    let mut excluder = ignore::gitignore::GitignoreBuilder::new(&root);
    for pat in exclude {
        excluder
            .add_line(None, pat)
            .map_err(|e| anyhow::anyhow!("invalid --exclude pattern `{pat}`: {e}"))?;
    }
    let excluder = excluder
        .build()
        .map_err(|e| anyhow::anyhow!("compiling --exclude patterns: {e}"))?;
    text.push_str(&gather_includes(
        &root,
        &includes,
        max,
        &excluder,
        &mut seen,
        &mut sources,
    )?);
    let git_truth = gather_git_truth(&root, &seen.included);
    text.push_str(&git_truth.text);
    sources.push(git_truth.source);
    let (rules_text, rules_active) = gather_rules(rule_sources, disabled_rules, &mut sources)?;
    text.push_str(&rules_text);
    if findings || recheck_findings {
        text.push_str(&gather_findings(&root, &mut sources, recheck_findings)?);
    }
    if agenda {
        text.push_str(&gather_agenda(&root, &mut sources)?);
    }
    if !from_runs.is_empty() {
        text.push_str(&gather_runs(from_runs, &mut sources)?);
    }

    let mut excluded = if includes.is_empty() {
        vec!["the repo's source files (--include <path> or --changed to add)".to_owned()]
    } else {
        vec![
            "the repo's other source files (beyond those added via --include/--changed)".to_owned(),
        ]
    };
    // Echo any --exclude patterns so a dropped slice is never a silent default.
    if !exclude.is_empty() {
        excluded.push(format!("--exclude: {}", exclude.join(", ")));
    }
    // Echo --no-scan so the skipped scan baseline is never a silent default.
    if !scan {
        excluded.push("the repository scan + detected manifests (--no-scan)".to_owned());
    }

    Ok(Context {
        text,
        sources,
        excluded,
        root,
        rules_active,
    })
}

/// The exact parameters a run used — reported alongside every result so nothing is opaque.
#[derive(Serialize)]
struct RunReport<'a> {
    model: String,
    /// Whether `model` is response-confirmed or the unconfirmed requested id (a backend whose events
    /// echo no model) — shown beside the model so the report never overstates the identity that ran.
    /// `None` on a dry run: nothing ran, so no source verdict exists (and none is serialized).
    #[serde(skip_serializing_if = "Option::is_none")]
    model_source: Option<crate::ai::ModelSource>,
    /// Synthesis backend that ran (`claude` | `codex`).
    backend: &'a str,
    /// How many synthesis runs were combined (1 = single; >1 = concurrent multi-run, unioned). The
    /// count that *succeeded* — compare with `runs_requested` to see whether any were dropped.
    runs: usize,
    /// How many runs were requested (`--runs N`). When it exceeds `runs`, some failed and were
    /// skipped — surfaced here so a `--json` consumer sees the drop in the payload, not just on stderr.
    runs_requested: usize,
    /// Arc-requested tool grants (currently Claude-only). This is not a claim that a provider has no
    /// built-in/managed tools; [`ai::RunBoundary::tool_runtime`] carries that separate fact.
    tools: Vec<&'a str>,
    /// The decomposed execution boundary: what Arc suppresses or grants, plus what still inherits.
    boundary: ai::RunBoundary,
    /// The hard cost cap in effect, or `None` — surfaced every run so an uncapped run says so.
    max_budget_usd: Option<f64>,
    /// Codex reasoning effort in effect, surfaced because it shapes cost; `None` for backends without it.
    reasoning_effort: Option<&'a str>,
    /// Whether the results were ordered by significance (`--ranked`).
    ranked: bool,
    /// Whether each result was labeled with a `kind` (`--kinds`).
    kinds: bool,
    /// Whether `--free-kinds` unlocked the taxonomy's enum on `kind`.
    free_kinds: bool,
    context: &'a [String],
    excluded: &'a [String],
    /// Active `.arc/settings.json` layers in effect for this run (empty = built-in defaults only).
    config: &'a [String],
    /// The findings field this run gates on (`--fail-on-findings`), or `None`. Recorded as a run
    /// parameter (for `--json`); the pass/block outcome is shown on its own line to stay legible.
    gate: Option<&'a str>,
}

impl RunReport<'_> {
    fn human(&self) -> String {
        let tools = if self.tools.is_empty() {
            "none".to_owned()
        } else {
            self.tools.join(",")
        };
        // Single-sourced fan-out + drop summary (shared with the stored-run replay).
        let runs = match crate::log::runs_summary(self.runs, self.runs_requested) {
            Some(s) => format!("  {s}"),
            None => String::new(),
        };
        // The cap's worst-case exposure is over the runs *attempted* (each can spend up to it before
        // failing), so size it by the requested count, not the surviving one.
        let budget = crate::log::budget_display(self.max_budget_usd, self.runs_requested);
        // Reasoning effort is cost-shaping, so surface it next to the backend that uses it.
        let reasoning = match self.reasoning_effort {
            Some(effort) => format!("  reasoning={effort}"),
            None => String::new(),
        };
        // A model id the response never confirmed is labeled, not presented as the ran identity.
        // (A dry run carries no source — the [dry run] banner already says nothing ran.)
        let model_source = match self.model_source {
            Some(crate::ai::ModelSource::Requested) => " (requested — backend echoes no model id)",
            Some(crate::ai::ModelSource::Reported) | None => "",
        };
        let mut line = format!(
            "model={}{model_source}{}  backend={}{}  tool_grants={}  ambient={}  budget={}{}{}  context=[{}]",
            self.model,
            runs,
            self.backend,
            reasoning,
            tools,
            if self.boundary.ambient { "on" } else { "off" },
            budget,
            if self.ranked { "  ranked" } else { "" },
            if self.kinds { "  kinds" } else { "" },
            self.context
                .iter()
                .map(|s| crate::display_path(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !self.excluded.is_empty() {
            line.push_str(&format!("  excluded=[{}]", self.excluded.join(", ")));
        }
        line.push_str(&format!("\nboundary: {}", self.boundary.human()));
        line.push_str(&format!(
            "\nconfig: {}",
            crate::join_or(self.config, crate::settings::NO_LAYERS)
        ));
        line
    }
}

#[derive(Serialize)]
struct SynthOutput<'a> {
    /// The run's id when logging is on — the key `arc log <id>` and the result store use.
    id: Option<String>,
    run: RunReport<'a>,
    synthesis: String,
    usage: ai::Usage,
    /// Path written by `--output`, if any (so the run report says where the doc went).
    output: Option<String>,
    /// Path of the run log this run was appended to, if logging was on (disclosed every run).
    log: Option<String>,
    /// Schema-validated structured result, present whenever the verb declares a shape.
    structured: Option<serde_json::Value>,
    /// An agent-reported failure (the run spent but didn't complete); absent on a normal completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// One line of `~/.arc/logs/runs.jsonl`: the durable, machine-readable trace of a real run — its
/// params, context sources, and ground-truth token usage + cost. Dry runs are never logged (no
/// spend, no call). The full [`ai::Usage`] is nested verbatim, so token/cost fields single-source.
#[derive(Serialize)]
struct RunRecord<'a> {
    /// Unique run id (`<ts>-<pid>-<nanos>`) — keys the full-result store at `~/.arc/logs/results/<id>.json`.
    id: String,
    ts: u64,
    /// The arc binary's version (single-sourced from Cargo.toml at compile time), so runs stay
    /// attributable to the binary that made them when logs aggregate into cross-version metrics.
    version: &'static str,
    /// The build's commit identity (short sha, `-dirty` for worktree builds, `unknown` outside
    /// git) — the fingerprint *below* `version`: two runs on one semver can come from different
    /// builds, and attribution must be able to tell them apart.
    build: &'static str,
    command: &'a str,
    repo: String,
    /// The repo's commit at run time (short sha, `-dirty` when the worktree exceeded it) — what the
    /// run actually judged, carried into promoted findings so their claims stay anchored to a code
    /// state. Omitted when the target isn't a git repo (absence is the disclosure).
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    model: &'a str,
    backend: &'a str,
    runs: usize,
    /// Runs requested (`--runs N`) — exceeds `runs` when some failed, so the durable trace shows the drop too.
    runs_requested: usize,
    boundary: ai::RunBoundary,
    max_budget_usd: Option<f64>,
    /// Codex reasoning effort recorded (cost-shaping); `None` for backends without it.
    reasoning_effort: Option<&'a str>,
    /// Arc-requested Claude tool grants (empty = none). Provider-owned tools are disclosed by
    /// `boundary`, not inferred from this list.
    tools: &'a [String],
    ranked: bool,
    kinds: bool,
    free_kinds: bool,
    structured: bool,
    /// Size of the assembled prompt arc sent, in characters — the deterministic context-size
    /// counterpart to the billed token ground truth nested in `usage`.
    prompt_chars: usize,
    sources: &'a [String],
    /// The active rules as (id, body-fingerprint) pairs — the structured exposure half of firing
    /// stats, attributing a finding to the rule *version* that was in play. Records predating this
    /// field have no exposure data; stats read them as unknown, never zero.
    rules: &'a [crate::rules::ActiveRule],
    /// The effective taxonomy as (kind, description-fingerprint) pairs — the same exposure record
    /// as `rules`, for the vocabulary lever ([`ActiveKind`]). Absent on records predating the
    /// field (unknown, never zero); `[]` = the run had no taxonomy in play.
    taxonomy: &'a [ActiveKind],
    usage: &'a ai::Usage,
    /// The findings field gated on (`--fail-on-findings`), or `None` — with `blocked`, this lets
    /// metrics ask "how often does the gate actually block?" (the spend-vs-value question).
    gate: Option<&'a str>,
    blocked: bool,
    /// The run's agent-reported failure, mirrored from [`ai::Synthesis::error`] into the durable record
    /// (`Some` ⇒ it spent but didn't complete; the real cost is in `usage`). Omitted from the JSON when
    /// absent, so existing records and the success case are unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// The full result of one run, written to `~/.arc/logs/results/<id>.json` so `arc log <id>` can
/// re-show it without re-running: the run record (metadata), the verbatim prompt sent, and the
/// synthesis content.
#[derive(Serialize)]
struct StoredRun<'a> {
    run: &'a RunRecord<'a>,
    /// The full prompt sent to the model (instruction + assembled context + notes), kept verbatim so
    /// a past run is fully inspectable/reproducible — the heavy field, which is why it lives here in
    /// the per-run result file, not in the compact one-line `runs.jsonl` record.
    prompt: &'a str,
    text: &'a str,
    structured: Option<&'a serde_json::Value>,
}

#[derive(Serialize)]
struct DryRunOutput<'a> {
    dry_run: bool,
    run: RunReport<'a>,
    estimate: ai::Estimate,
    note: &'static str,
    /// Where `--output` *would* write on a real run, if set.
    output_target: Option<String>,
    /// The human preview header (params + estimate + disclosures — the text before the prompt), as a
    /// machine-readable field: a consumer showing the preview (the TUI's launch gate) reads it here
    /// instead of parsing the human layout, while its composition keeps this one home.
    preview: String,
    prompt: &'a str,
}

/// The human display body for a synthesis result: the structured output pretty-printed when present
/// (a null value counts as absent), else the prose `text`. One definition for the live run report
/// and the stored-run replay, so they can't diverge. A `serde_json::Value` always re-serializes
/// (string keys), so the pretty-print is infallible — asserted, as in [`combine_runs`].
pub(crate) fn body_display(structured: Option<&serde_json::Value>, text: &str) -> String {
    let Some(value) = structured.filter(|v| !v.is_null()) else {
        // No structure — a prose verb (summarize), where the narrative text *is* the product.
        return text.to_owned();
    };
    // The typed results are canonical and the human view derives from them — each item's fields as
    // labelled lines, then the run's `note` — never a second, driftable prose account. A structured
    // value outside the results envelope falls back to pretty JSON so nothing is hidden (infallible:
    // a `serde_json::Value` always re-serializes, asserted as in [`combine_runs`]).
    let (Some(results), Some(note)) = (
        value.get(RESULTS_KEY).and_then(serde_json::Value::as_array),
        value.get(NOTE_KEY).and_then(serde_json::Value::as_str),
    ) else {
        return serde_json::to_string_pretty(value).expect("a serde_json::Value re-serializes");
    };
    let mut out = format!("{} result(s)", results.len());
    for item in results {
        out.push_str("\n\n");
        out.push_str(&item_bullets(item));
    }
    out.push_str(&format!("\n\nnote: {note}"));
    out
}

/// One structured item's fields as `- **key:** value` lines — the human rendering of a typed result,
/// shared by the run output ([`body_display`]) and promote's ledger entries, so a finding reads the
/// same in the terminal, the TUI, and the ledger.
pub(crate) fn item_bullets(item: &serde_json::Value) -> String {
    item.as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            let val = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            format!("- **{k}:** {val}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Preview (dry-run) or run a synthesis prompt, echoing the full run parameters.
pub fn run(prompt: &str, opts: &SynthOptions) -> anyhow::Result<ExitCode> {
    // The model was resolved by the backend (per-backend defaults live there, not here).
    let requested = opts.model;
    // The report names the model that actually ran (set from the response after the call); until
    // then it holds the requested model — all a dry run can name, since nothing runs.
    let boundary = ai::backend(opts.backend)?.boundary(opts.ambient_memory, opts.allowed_tools);
    let mut report = RunReport {
        model: requested.to_owned(),
        // No source verdict until a response exists — a real run sets it from the usage; a dry run
        // never does (nothing runs).
        model_source: None,
        backend: opts.backend,
        runs: opts.runs,
        runs_requested: opts.runs,
        tools: opts.allowed_tools.iter().map(String::as_str).collect(),
        boundary,
        max_budget_usd: opts.max_budget_usd,
        reasoning_effort: opts.reasoning_effort,
        ranked: opts.ranked,
        kinds: opts.kinds,
        free_kinds: opts.free_kinds,
        context: opts.sources,
        excluded: opts.excluded,
        config: opts.config,
        gate: opts.gate,
    };

    if opts.dry_run {
        let estimate = ai::estimate(prompt);
        let output_target = opts
            .output
            .map(|dir| output_path(dir, opts.command).display().to_string());
        let mut preview = format!(
            "[dry run]\nrun: {}\nprompt: {} chars (~{} tokens)\nnote: {}",
            report.human(),
            estimate.chars,
            estimate.approx_tokens,
            DRY_RUN_NOTE,
        );
        if let Some(target) = &output_target {
            preview.push_str(&format!("\noutput: would write {target} on a real run"));
        }
        if opts.schema.is_some() {
            preview.push_str("\nstructured: on — the result will be a schema-validated object");
        }
        if let Some(field) = opts.gate {
            preview.push_str(&format!(
                "\ngate: on — a real run exits {GATE_BLOCKED_EXIT} if `{field}` is non-empty"
            ));
        }
        // Disclose logging status + where real runs would record, even though a dry run logs nothing.
        match (opts.log, crate::log::path()) {
            (true, Some(path)) => {
                preview.push_str(&format!(
                    "\nlogging: on — real runs append to {}",
                    path.display()
                ));
            }
            (false, _) => preview.push_str(LOGGING_OFF_NOTE),
            (true, None) => {}
        }
        let human = format!("{preview}\n\n{prompt}");
        let out = DryRunOutput {
            dry_run: true,
            run: report,
            estimate,
            note: DRY_RUN_NOTE,
            output_target,
            preview,
            prompt,
        };
        emit(&out, &human, opts.json)?;
        return Ok(ExitCode::SUCCESS);
    }

    let (synthesis, runs) = if opts.runs > 1 {
        multi_synthesize(prompt, requested, opts)?
    } else {
        (synthesize_run(prompt, requested, opts, 0)?, 1)
    };
    let usage = synthesis.usage;
    // From here the report reflects the model the response says ran — or, disclosed via
    // model_source, the requested id where the backend echoes none — and the combined run count.
    report.model = usage.model.clone();
    report.model_source = Some(usage.model_source);
    report.runs = runs;
    let structured = synthesis.structured;
    let text = synthesis.text;
    // For an errored synthesis ([`ai::Synthesis::error`]), the gate and `--output` don't apply and the
    // body is the failure itself — but it's still logged below (with its real spend) and exits non-zero.
    let mut errored = synthesis.error;
    // Content conformance is the provider's, enforced natively at the channel (constrained
    // decoding) — arc does not re-judge it. What arc does insist on is that the certified artifact
    // *arrived*: a schema-requested run that ends with no structured payload at all didn't deliver
    // a verdict, and its prose must not stand in for the typed result. That failure joins the
    // errored-run path with its real spend — an errored run can never pass a gate.
    if let (Some(_), None, None) = (opts.schema, structured.as_ref(), &errored) {
        errored = Some(
            "the run returned no structured output for its declared schema — prose can't stand in for the typed result".to_owned(),
        );
    }
    // Count the gated findings — the one read a push decision acts on. An absent `results` array is
    // an incomplete payload, and by this point the synthesis has *spent*, so it joins the
    // errored-run path (logged with its real usage, non-zero exit) rather than bailing — and never
    // reads as a 0-findings pass: refusing to invent a default at this read is what keeps an
    // incomplete run from certifying a push.
    let gate_findings = match (errored.is_some(), opts.gate) {
        (false, Some(field)) => match structured
            .as_ref()
            .and_then(|v| v.get(field))
            .and_then(serde_json::Value::as_array)
        {
            Some(items) => Some(items.len()),
            None => {
                errored = Some(format!(
                    "gated on `{field}` but the result has no `{field}` array — the run's payload is incomplete"
                ));
                None
            }
        },
        _ => None,
    };
    let is_errored = errored.is_some();
    let cost = crate::log::cost_or_unavailable(usage.cost_usd);
    let body = match &errored {
        Some(error) => format!("error: {error}"),
        None => body_display(structured.as_ref(), &text),
    };
    // A gated fan-out (`--fail-on-findings --runs N`) that lost runs can't certify "clean" — the
    // dropped runs' coverage is missing — so fail closed: block even when the survivors found nothing.
    // (`runs` is the surviving count; a failed *or* errored child reduces it below `opts.runs`.)
    let incomplete_gate = opts.gate.is_some() && runs < opts.runs;
    let gate_blocked = gate_findings.is_some_and(|n| n > 0) || incomplete_gate;
    // --output: also write the result as a doc with a provenance header (a completed run only — a
    // failed run has no result body to write).
    let written = match (is_errored, opts.output) {
        // --output writes an auxiliary provenance doc; a failure here must not abort the billed run
        // before its cost is logged and the result emitted — warn and proceed (best-effort, like the
        // run log itself), rather than discarding already-spent work.
        (false, Some(dir)) => {
            match write_output(
                dir,
                opts.command,
                &body,
                &report.model,
                usage.model_source,
                opts.sources.len(),
                &cost,
            ) {
                Ok(path) => Some(path),
                Err(e) => {
                    eprintln!("arclite: --output doc not written ({e:#})");
                    None
                }
            }
        }
        _ => None,
    };
    let mut human = format!(
        "{}\n\nrun: {}\ncost: {}",
        body,
        report.human(),
        crate::log::usage_display(
            usage.input_tokens,
            usage.cache_creation_input_tokens,
            usage.cache_read_input_tokens,
            usage.output_tokens,
            usage.cost_usd,
            usage.cost_partial,
        ),
    );
    if let Some(path) = &written {
        human.push_str(&format!("\nwrote: {}", path.display()));
    }
    // Gate outcome on its own line so a blocked commit is unmistakable (real run only — the dry-run
    // path notes that gating is armed instead). Shown for pass too, so "on and clean" isn't silent.
    if let (Some(field), Some(n)) = (opts.gate, gate_findings) {
        let incomplete = if incomplete_gate {
            format!(
                " · {}/{} runs failed, failing closed",
                opts.runs - runs,
                opts.runs
            )
        } else {
            String::new()
        };
        human.push_str(&format!(
            "\ngate: {} — {n} `{field}`{incomplete}{}",
            crate::log::gate_label(gate_blocked),
            if gate_blocked {
                format!(" (exit {GATE_BLOCKED_EXIT})")
            } else {
                String::new()
            },
        ));
    }
    // Append a durable run record (real runs only) before emitting — observability that outlives
    // the terminal scrollback. A logging failure warns but never fails the command.
    let (logged, id) = if opts.log {
        let ts = crate::log::now_secs();
        // `<secs>-<pid>-<nanos>`: the trailing nanos guard against a collision when two runs share a
        // second and a pid (a reused pid across concurrent sessions on the shared `~/.arc`), which would
        // otherwise overwrite one run's stored result with another's.
        let id = format!(
            "{ts}-{}-{}",
            std::process::id(),
            crate::log::now_subsec_nanos()
        );
        let record = RunRecord {
            id: id.clone(),
            ts,
            version: env!("CARGO_PKG_VERSION"),
            build: env!("ARC_BUILD_COMMIT"),
            command: opts.command,
            // The recorded form promote/retire later reopen as a path — the shared lossless
            // conversion, not display formatting (confine-display-formatting-to-output).
            repo: crate::log::repo_record_string(opts.dir),
            commit: repo_commit(opts.dir),
            model: &report.model,
            backend: opts.backend,
            runs,
            runs_requested: opts.runs,
            boundary: report.boundary,
            max_budget_usd: opts.max_budget_usd,
            reasoning_effort: opts.reasoning_effort,
            tools: opts.allowed_tools,
            ranked: opts.ranked,
            kinds: opts.kinds,
            free_kinds: opts.free_kinds,
            structured: opts.schema.is_some(),
            prompt_chars: prompt.chars().count(),
            sources: opts.sources,
            rules: opts.rules_active,
            taxonomy: opts.taxonomy,
            usage: &usage,
            gate: opts.gate,
            blocked: gate_blocked,
            error: errored.as_deref(),
        };
        let logged = crate::log::append(&record);
        // Store the full result (best-effort) so `arc log <id>` can re-show it without re-running.
        crate::log::store_result(
            &id,
            &StoredRun {
                run: &record,
                prompt,
                text: &text,
                structured: structured.as_ref(),
            },
        );
        (logged, Some(id))
    } else {
        (None, None)
    };
    // Disclose where the run was logged and its id (`arc log <id>` re-shows it), or that logging is
    // off — the log location is never hidden.
    match (&logged, &id) {
        (Some(path), Some(id)) => {
            human.push_str(&format!(
                "\nlogged: {} · id {id}",
                crate::display_path(&path.display().to_string())
            ));
        }
        (None, _) if !opts.log => human.push_str(LOGGING_OFF_NOTE),
        _ => {} // logging on but the append failed — append() already warned to stderr
    }
    let out = SynthOutput {
        id,
        run: report,
        synthesis: text,
        usage,
        output: written.map(|p| p.display().to_string()),
        log: logged.map(|p| p.display().to_string()),
        structured,
        error: errored,
    };
    emit(&out, &human, opts.json)?;
    // The gate's verdict is the process exit code (distinct from error) so a hook enforces on status
    // alone; SUCCESS when not gating or when the findings collection is empty.
    Ok(if is_errored {
        ExitCode::from(ERRORED_EXIT)
    } else if gate_blocked {
        ExitCode::from(GATE_BLOCKED_EXIT)
    } else {
        ExitCode::SUCCESS
    })
}

/// Register one run in the active-run registry and synthesize it — the one invocation the single-run
/// path and the `--runs N` fan-out share, so the parameter list can't drift between them. The marker
/// records live progress for `arc status` and clears when its `Active` guard drops.
fn synthesize_run(
    prompt: &str,
    model: &str,
    opts: &SynthOptions,
    index: usize,
) -> anyhow::Result<ai::Synthesis> {
    let active = crate::runs::register(opts.command, opts.dir, model, index);
    if active.is_none() {
        // Live tracking is best-effort, but a failure shouldn't be silent — say so (as logging warns
        // when its write fails) rather than leave the user wondering why `arc status` shows nothing.
        eprintln!(
            "arclite: this run won't appear in `arc status` (couldn't write its registry marker)"
        );
    }
    ai::backend(opts.backend)?.synthesize(
        &ai::Request {
            prompt,
            model,
            allowed_tools: opts.allowed_tools,
            dir: opts.dir,
            ambient_memory: opts.ambient_memory,
            json_schema: opts.schema,
            max_budget_usd: opts.max_budget_usd,
            reasoning_effort: opts.reasoning_effort,
        },
        active,
    )
}

/// Run the synthesis `opts.runs` times concurrently and combine the outcomes, returning the combined
/// result and how many runs succeeded. A failed or errored run is surfaced and skipped from the result
/// union; an all-errored fan-out still returns an errored synthesis carrying the spent cost, and only an
/// all-*outright*-failed fan-out (no run returned a synthesis, so no spend to preserve) errors.
fn multi_synthesize(
    prompt: &str,
    model: &str,
    opts: &SynthOptions,
) -> anyhow::Result<(ai::Synthesis, usize)> {
    let n = opts.runs;
    let outcomes: Vec<anyhow::Result<ai::Synthesis>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..n)
            .map(|index| scope.spawn(move || synthesize_run(prompt, model, opts, index)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a synthesis thread panicked"))
            .collect()
    });

    let mut ok = Vec::new();
    let mut errored = Vec::new(); // ran and *spent* but reported an error
    for outcome in outcomes {
        match outcome {
            Ok(s) if s.error.is_none() => ok.push(s),
            // A run that ran but reported an error (e.g. a tripped cap) is excluded from the result
            // union (like an outright failure) and surfaced — but it *spent*, so it's kept for the cost
            // fold below rather than having its tokens silently dropped from the fan-out total.
            Ok(s) => {
                eprintln!(
                    "arclite: a run reported an error and was skipped: {}",
                    s.error.as_deref().unwrap_or_default()
                );
                errored.push(s);
            }
            Err(e) => eprintln!("arclite: a run failed and was skipped: {e:#}"),
        }
    }
    let succeeded = ok.len();
    if ok.is_empty() {
        // No run produced a usable result. If some ran-but-errored they still *spent*: return a combined
        // errored synthesis carrying their folded usage, so the caller's logging path records the fan-out's
        // real cost — the single-run errored path and the "a failed run's real cost is recorded rather
        // than lost" guarantee — instead of erroring out before that block and dropping the spend. Only an
        // all-*outright*-failed fan-out (every run returned `Err`: no synthesis, no spend to preserve) has
        // nothing to return and errors here.
        anyhow::ensure!(!errored.is_empty(), "all {n} runs failed");
        let usage = sum_usage(errored.iter().map(|s| &s.usage));
        let mut combined = errored
            .into_iter()
            .next()
            .expect("errored is non-empty (just ensured)");
        combined.usage = usage;
        return Ok((combined, succeeded)); // succeeded == 0: an errored result, surfaced and logged with cost
    }
    // The whole fan-out's spend, summed up front: if the union below fails, this is what the errored
    // record carries — completed calls' usage must reach the log even when their results can't be
    // combined (account-for-consumed-cost-on-failure).
    let total_usage = sum_usage(ok.iter().chain(errored.iter()).map(|s| &s.usage));
    let mut combined = match combine_runs(ok) {
        Ok(combined) => combined,
        Err(e) => {
            return Ok((
                ai::Synthesis {
                    text: String::new(),
                    usage: total_usage,
                    structured: None,
                    error: Some(format!("the fan-out's results couldn't be combined: {e:#}")),
                },
                succeeded,
            ));
        }
    };
    // The combined result carries the whole fan-out's usage — the errored-but-spent children's
    // results are excluded, their cost is not.
    combined.usage = total_usage;
    Ok((combined, succeeded))
}

/// Combine successful runs: sum their usage, then union the structured `results` (deduped) — or, for
/// prose commands, present each run's text in turn.
fn combine_runs(runs: Vec<ai::Synthesis>) -> anyhow::Result<ai::Synthesis> {
    let usage = sum_usage(runs.iter().map(|r| &r.usage));
    // Structured iff *every* run produced structured output; one prose run and the whole batch is
    // presented as prose. Collecting `Option<&Value>`s into `Option<Vec<_>>` expresses that all-or-
    // prose split in one step — no separate `all(is_some)` guard, no filter that can never filter.
    if let Some(structured) = runs
        .iter()
        .map(|r| r.structured.as_ref())
        .collect::<Option<Vec<_>>>()
    {
        let combined = union_results(structured.into_iter())?;
        let text =
            serde_json::to_string_pretty(&combined).expect("a serde_json::Value re-serializes");
        Ok(ai::Synthesis {
            text,
            usage,
            structured: Some(combined),
            error: None,
        })
    } else {
        let text = runs
            .iter()
            .enumerate()
            .map(|(i, r)| format!("— run {} —\n{}", i + 1, r.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(ai::Synthesis {
            text,
            usage,
            structured: None,
            error: None,
        })
    }
}

/// Union the `results` arrays of several structured outputs into one, and carry every run's `note`
/// (labeled per run when there is more than one). Only byte-identical items collapse — a near-no-op
/// in practice, since independent runs rarely emit the same prose verbatim; judging when two
/// findings are the same *in substance* is the open semantic combine. Generic over the item shape,
/// so it serves repeats of one command and (later) different commands.
fn union_results<'a>(
    structured: impl Iterator<Item = &'a serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    let mut pooled: Vec<serde_json::Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for value in structured {
        // Both fields are guaranteed present by the validated envelope schema ([`results_schema`]
        // marks them required); a missing one means the CLI ignored the schema, surfaced as an error
        // the same way the gate path treats it — never silently dropped (which would under-report).
        let items = value
            .get(RESULTS_KEY)
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "a run result has no `{RESULTS_KEY}` array (the CLI ignored the schema)"
                )
            })?;
        for item in items {
            if !pooled.contains(item) {
                pooled.push(item.clone());
            }
        }
        let note = value
            .get(NOTE_KEY)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "a run result has no `{NOTE_KEY}` string (the CLI ignored the schema)"
                )
            })?;
        notes.push(note.to_owned());
    }
    let note = if notes.len() == 1 {
        notes.remove(0)
    } else {
        notes
            .iter()
            .enumerate()
            .map(|(i, n)| format!("run {}: {n}", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut obj = serde_json::Map::new();
    obj.insert(RESULTS_KEY.to_owned(), serde_json::Value::Array(pooled));
    obj.insert(NOTE_KEY.to_owned(), serde_json::Value::String(note));
    Ok(serde_json::Value::Object(obj))
}

/// Sum token usage and cost across several runs' usages. The caller guarantees at least one (a
/// fan-out always keeps ≥1 surviving run).
fn sum_usage<'a>(usages: impl Iterator<Item = &'a ai::Usage>) -> ai::Usage {
    let mut usages = usages;
    let mut total = usages
        .next()
        .expect("sum_usage needs at least one usage")
        .clone();
    for u in usages {
        total.input_tokens += u.input_tokens;
        total.output_tokens += u.output_tokens;
        total.cache_creation_input_tokens += u.cache_creation_input_tokens;
        total.cache_read_input_tokens += u.cache_read_input_tokens;
        // The identity that ran: members *can* differ (an errored member reports its requested id;
        // a substitution could differ mid-fan-out), so the identity *sets* merge — structured data,
        // not a re-split of the joined display string — and never the first member's id presented
        // as though one model handled every run. Confidence folds to the weakest: any
        // requested-only member makes the whole total requested-only.
        for m in &u.models {
            if !total.models.contains(m) {
                total.models.push(m.clone());
            }
        }
        if u.model_source == crate::ai::ModelSource::Requested {
            total.model_source = crate::ai::ModelSource::Requested;
        }
        // A member whose spend is unknown makes the total's zeros partial too — carried so the
        // rollup can't read the sum as fully-measured.
        total.spend_unknown = total.spend_unknown || u.spend_unknown;
        // Dollar cost folds as the sum of the members that reported one — `None` only when *no*
        // member did (codex fan-outs) — so a single member with an unreported cost (a claude error
        // payload without `total_cost_usd`) can't erase the survivors' known spend. That sum is a
        // *lower bound*, and it says so: `cost_partial` marks the mix, displays as "≥", and rides
        // the record — a partial total is never presented as exact.
        total.cost_partial = total.cost_partial
            || u.cost_partial
            || (total.cost_usd.is_some() != u.cost_usd.is_some());
        total.cost_usd = match (total.cost_usd, u.cost_usd) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
        };
    }
    // The display form re-derives from the merged set — one direction (data → display), never
    // back — through the same join `parse_result` uses, so the separator has one home.
    total.model = crate::ai::join_models(&total.models);
    total
}

/// The path `--output` writes to: `<dir>/<command>.md`. One definition so the dry-run preview and the
/// real `write_output` agree on where a run would write (preview-must-share-execution-path).
fn output_path(dir: &Path, command: &str) -> PathBuf {
    dir.join(format!("{command}.md"))
}

/// Write the synthesis to `<dir>/<command>.md` with a provenance header (model, sources, cost).
/// Returns the path.
fn write_output(
    dir: &Path,
    command: &str,
    text: &str,
    model: &str,
    model_source: crate::ai::ModelSource,
    n_sources: usize,
    cost: &str,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("could not create output dir {}: {e}", dir.display()))?;
    let path = output_path(dir, command);
    let bin = crate::cli::binary_name();
    // Provenance must not overstate: a requested-not-confirmed model id carries its marker into
    // the generated doc, the same disclosure the live report and the log make.
    let model_note = match model_source {
        crate::ai::ModelSource::Reported => "",
        crate::ai::ModelSource::Requested => crate::log::MODEL_REQUESTED_SUFFIX,
    };
    let body = format!(
        "<!-- Generated by `{bin} run {command}` — model={model}{model_note}, {n_sources} context source(s), cost {cost}. Self-derived; do not hand-maintain. -->\n\n{text}\n"
    );
    std::fs::write(&path, &body)
        .map_err(|e| anyhow::anyhow!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    const GIT_TRUTH_MARKER: &str =
        "\nVersion-control truth (JSON; path names and metadata only):\n";

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "arclite-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create isolated test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("run git in test repository");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git(root: &Path) {
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "Arclite Test"]);
        git(
            root,
            &["config", "user.email", "arclite-test@example.invalid"],
        );
        git(root, &["config", "commit.gpgsign", "false"]);
        git(root, &["config", "core.hooksPath", ".git/no-hooks"]);
    }

    fn context(root: &Path, includes: &[PathBuf]) -> Context {
        gather_context(
            root,
            &ContextSpec {
                includes,
                rule_sources: &[],
                disabled_rules: &[],
                max: None,
                changed: false,
                exclude: &[],
                scan: false,
                findings: false,
                recheck_findings: false,
                agenda: false,
                from_runs: &[],
            },
        )
        .expect("gather deterministic test context")
    }

    fn git_truth(ctx: &Context) -> serde_json::Value {
        let start = ctx
            .text
            .rfind(GIT_TRUTH_MARKER)
            .expect("context carries Git truth")
            + GIT_TRUTH_MARKER.len();
        serde_json::from_str(ctx.text[start..].trim()).expect("Git truth is valid JSON")
    }

    fn included_state<'a>(truth: &'a serde_json::Value, path: &str) -> &'a str {
        truth["included_repository_files"]
            .as_array()
            .expect("included files array")
            .iter()
            .find(|row| row["path"].as_str() == Some(path))
            .and_then(|row| row["git_state"].as_str())
            .expect("included path has a Git state")
    }

    fn string_array_contains(value: &serde_json::Value, key: &str, needle: &str) -> bool {
        value[key]
            .as_array()
            .expect("Git path-set array")
            .iter()
            .any(|path| path.as_str() == Some(needle))
    }

    #[test]
    fn git_truth_closes_the_referenced_ignored_file_gap() {
        let dir = TestDir::new("git-truth");
        init_git(dir.path());

        std::fs::write(dir.path().join(".gitignore"), "*.pfx\n").unwrap();
        std::fs::write(
            dir.path().join("project.csproj"),
            "<SignAssemblyKeyFile>signing.pfx</SignAssemblyKeyFile>\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("dirty.txt"), "committed\n").unwrap();
        std::fs::write(dir.path().join("tracked.log"), "tracked\n").unwrap();
        std::fs::create_dir(dir.path().join("alias")).unwrap();
        git(dir.path(), &["add", "--", "."]);
        git(dir.path(), &["commit", "-qm", "fixture"]);

        // A tracked file remains tracked when a later ignore rule matches its name.
        std::fs::write(dir.path().join(".gitignore"), "*.pfx\n*.log\n").unwrap();
        git(dir.path(), &["add", "--", ".gitignore"]);
        git(dir.path(), &["commit", "-qm", "ignore logs"]);

        std::fs::write(dir.path().join("dirty.txt"), "working tree change\n").unwrap();
        std::fs::write(dir.path().join("signing.pfx"), "IGNORED-SECRET-CONTENTS").unwrap();
        std::fs::write(dir.path().join("loose.txt"), "ordinary untracked\n").unwrap();

        let includes = vec![
            PathBuf::from("alias/../project.csproj"),
            PathBuf::from("dirty.txt"),
            PathBuf::from("tracked.log"),
        ];
        let ctx = context(dir.path(), &includes);
        let truth = git_truth(&ctx);

        assert!(ctx.text.contains("signing.pfx</SignAssemblyKeyFile>"));
        assert!(!ctx.text.contains("IGNORED-SECRET-CONTENTS"));
        assert_eq!(
            included_state(&truth, "project.csproj"),
            "index_tracked_clean"
        );
        assert_eq!(included_state(&truth, "dirty.txt"), "index_tracked_dirty");
        assert_eq!(included_state(&truth, "tracked.log"), "index_tracked_clean");
        assert!(string_array_contains(
            &truth,
            "ignored_untracked_present",
            "signing.pfx"
        ));
        assert!(string_array_contains(
            &truth,
            "untracked_present",
            "loose.txt"
        ));
        assert!(string_array_contains(&truth, "changed_paths", "dirty.txt"));
        assert!(
            truth["claim_rule"]
                .as_str()
                .is_some_and(|rule| rule.contains("merely mentioned"))
        );
        assert!(ctx.sources.iter().any(|source| {
            source.contains("git state: path names only")
                && source.contains("snapshot adds no file contents")
        }));

        // On a case-insensitive filesystem, preserve Git's canonical index spelling rather than
        // turning a readable case-only alias into an unknown path. Case-sensitive hosts skip this.
        let case_alias = PathBuf::from("PROJECT.CSPROJ");
        if dir.path().join(&case_alias).try_exists().unwrap() {
            let alias_ctx = context(dir.path(), &[case_alias]);
            assert_eq!(
                included_state(&git_truth(&alias_ctx), "project.csproj"),
                "index_tracked_clean"
            );
        }
    }

    #[test]
    fn target_root_sentinels_classify_untracked_and_ignored_descendants() {
        let dir = TestDir::new("git-root-sentinel");
        init_git(dir.path());
        let ignored_root = dir.path().join("ignored-scope");
        std::fs::create_dir(&ignored_root).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "/ignored-scope/\n").unwrap();
        std::fs::write(ignored_root.join("tracked.txt"), "force tracked\n").unwrap();
        git(dir.path(), &["add", "--", ".gitignore"]);
        git(
            dir.path(),
            &["add", "-f", "--", "ignored-scope/tracked.txt"],
        );
        git(dir.path(), &["commit", "-qm", "fixture"]);
        std::fs::write(ignored_root.join("ignored.txt"), "ignored\n").unwrap();

        let ignored_ctx = context(
            &ignored_root,
            &[PathBuf::from("tracked.txt"), PathBuf::from("ignored.txt")],
        );
        let ignored_truth = git_truth(&ignored_ctx);
        assert_eq!(
            included_state(&ignored_truth, "tracked.txt"),
            "index_tracked_clean"
        );
        assert_eq!(
            included_state(&ignored_truth, "ignored.txt"),
            "ignored_untracked"
        );
        assert!(string_array_contains(
            &ignored_truth,
            "ignored_untracked_present",
            "."
        ));

        let untracked_root = dir.path().join("untracked-scope");
        std::fs::create_dir(&untracked_root).unwrap();
        std::fs::write(untracked_root.join("note.txt"), "untracked\n").unwrap();
        let untracked_ctx = context(&untracked_root, &[PathBuf::from("note.txt")]);
        let untracked_truth = git_truth(&untracked_ctx);
        assert_eq!(included_state(&untracked_truth, "note.txt"), "untracked");
        assert!(string_array_contains(
            &untracked_truth,
            "untracked_present",
            "."
        ));
    }

    #[cfg(unix)]
    #[test]
    fn parent_after_symlink_cannot_borrow_a_tracked_paths_state() {
        use std::os::unix::fs::symlink;

        let repo = TestDir::new("git-symlink-parent");
        init_git(repo.path());
        std::fs::write(repo.path().join("project.csproj"), "tracked copy\n").unwrap();
        git(repo.path(), &["add", "--", "project.csproj"]);
        git(repo.path(), &["commit", "-qm", "fixture"]);

        let external = TestDir::new("git-symlink-parent-external");
        std::fs::create_dir(external.path().join("subdir")).unwrap();
        std::fs::write(external.path().join("project.csproj"), "external copy\n").unwrap();
        symlink(external.path().join("subdir"), repo.path().join("alias")).unwrap();

        // Filesystem resolution follows `alias` before applying `..`, so this reads the external copy,
        // not the tracked file with the same basename in the repository.
        let ctx = context(repo.path(), &[PathBuf::from("alias/../project.csproj")]);
        assert!(ctx.text.contains("external copy"));
        let truth = git_truth(&ctx);
        assert!(
            truth["included_repository_files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["git_state"] == "outside_target")
        );
        assert!(
            !truth["included_repository_files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["git_state"] == "index_tracked_clean")
        );
    }

    #[test]
    fn non_git_context_marks_tracking_unknown() {
        let dir = TestDir::new("no-git-truth");
        std::fs::write(dir.path().join("included.txt"), "plain directory\n").unwrap();
        let ctx = context(dir.path(), &[PathBuf::from("included.txt")]);
        let truth = git_truth(&ctx);

        assert_eq!(truth["available"], false);
        assert_eq!(included_state(&truth, "included.txt"), "unknown");
        assert!(
            truth["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("not a Git work tree"))
        );
        assert!(
            ctx.sources.iter().any(
                |source| source == "git state: unavailable (make no tracking or commit claims)"
            )
        );
    }
}
