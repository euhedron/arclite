//! `arc builds` — the lifecycle surface for the git worktrees `arc run build` produces (one per
//! agenda item). It lists each build worktree with a *derived* state and acts on it: resume an
//! interrupted one, discard one, or clean out the merged ones (resolving their items).
//!
//! State is derived, never stamped. For an item's one worktree it reports the *latest run's* outcome
//! — a fact about that run's record, deliberately not a live diff of the tree (once a run completes,
//! the worktree is the operator's to review and amend; those amendments are their review in progress,
//! not a new state). The signals are read in priority order and are independent, so every worktree
//! lands in exactly one state:
//!   1. a live run marker for the item  → `InFlight`
//!   2. the build branch merged into the default branch (by patch-id) → `Merged`
//!   3. the latest `build` run's `outcome` → `completed` + commit → `PendingReview`
//!   4. otherwise (errored, or no final record at all) → `Interrupted`

use crate::cli::BuildsArgs;
use crate::cli::GlobalArgs;
use anyhow::Context as _;
use serde::Serialize;
use std::path::Path;

/// A build worktree's derived lifecycle state. Exactly one per worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum BuildState {
    InFlight,
    Merged,
    PendingReview,
    Interrupted,
}

impl BuildState {
    fn label(self) -> &'static str {
        match self {
            BuildState::InFlight => "in flight",
            BuildState::Merged => "merged",
            BuildState::PendingReview => "pending review",
            BuildState::Interrupted => "interrupted",
        }
    }
}

/// One build worktree and its derived state.
#[derive(Serialize)]
struct BuildEntry {
    item: String,
    state: BuildState,
    branch: String,
    worktree: String,
    /// The committed short sha of the latest run, when the record carries one (pending-review/merged).
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
}

#[derive(Serialize)]
struct BuildsReport {
    builds: Vec<BuildEntry>,
    /// Item-id subdirectories whose name is not a usable id (disclosed, never silently skipped).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unreadable: Vec<String>,
}

/// The `arc/build/<item>` branch convention — single-sourced with `commands::run_build` via this
/// one helper so the manager and the creator can't drift on the branch name.
pub(crate) fn build_branch(item: &str) -> String {
    format!("arc/build/{item}")
}

/// Derive the state of the one build worktree for `item`, in the documented priority order. Earlier
/// runs for the item are ledger history, never state inputs, so only the *latest* build run's outcome
/// is consulted (and only after the live-marker and merged signals fall through).
/// The shared inputs every state derivation reads — resolved once per `arc builds` invocation and
/// passed as one, so a new signal is a field here rather than another positional argument each
/// action helper must re-thread in order.
struct StateInputs<'a> {
    default_head: &'a str,
    /// This repo's recorded form — scopes the device-wide run log and the live markers to it.
    repo_record: &'a str,
    live_items: &'a [String],
    records: &'a [serde_json::Value],
}

fn derive_state(
    root: &Path,
    item: &str,
    s: &StateInputs,
) -> anyhow::Result<(BuildState, Option<String>)> {
    // 1. A live marker for this item — a running agent, never double-driven.
    if s.live_items.iter().any(|i| i == item) {
        return Ok((BuildState::InFlight, None));
    }
    let latest = latest_build_run(s.records, s.repo_record, item);
    let completed = matches!(&latest, Some(l) if l.outcome == "completed" && l.commit.is_some());
    let branch = build_branch(item);
    // 2. Merged into the default branch. Patch-equivalence covers a rebased or squashed merge; a
    // reachable tip with an empty cherry range is an ordinary fast-forward/merge-commit integration
    // *or* a branch that never gained a commit — topology can't tell them apart, so the run record
    // (guaranteed: build refuses to run unlogged) resolves it: a completed build there is merged,
    // an errored/absent one never landed anything and falls through to interrupted.
    let branch_present = crate::synth::build_branch_exists(root, &branch)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("checking the build branch for `{item}`"))?;
    if branch_present {
        let merged = match crate::synth::build_branch_merge_signal(root, &branch, s.default_head)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("checking whether `{item}` is merged"))?
        {
            crate::synth::MergeSignal::PatchEquivalent => true,
            crate::synth::MergeSignal::TipReachable => completed,
            crate::synth::MergeSignal::Unmerged => false,
        };
        if merged {
            return Ok((BuildState::Merged, latest.and_then(|l| l.commit)));
        }
    }
    // 3. The latest build run's outcome; 4. otherwise interrupted.
    if completed {
        Ok((BuildState::PendingReview, latest.and_then(|l| l.commit)))
    } else {
        Ok((BuildState::Interrupted, None))
    }
}

/// The latest `build` run record for `item` *in this repo*, read newest-first from the `build_run`
/// slice `arc run build` writes into `runs.jsonl` — structured fields, never reparsed prose. The
/// run id rides along for the resolution trail `--clean` writes. The log is device-wide, so the
/// record's `repo` must match: two repos can hold an item with the same id, and the other repo's
/// newer run must not determine this repo's state or provenance.
struct LatestBuild {
    run_id: Option<String>,
    outcome: String,
    commit: Option<String>,
}

fn latest_build_run(
    records: &[serde_json::Value],
    repo_record: &str,
    item: &str,
) -> Option<LatestBuild> {
    records.iter().find_map(|r| {
        (r.get("command").and_then(serde_json::Value::as_str) == Some("build")
            && crate::log::record_repo(r) == Some(repo_record)
            && r.pointer("/build_run/item")
                .and_then(serde_json::Value::as_str)
                == Some(item))
        .then(|| LatestBuild {
            run_id: r
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            outcome: r
                .pointer("/build_run/outcome")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            commit: r
                .pointer("/build_run/commit")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    })
}

/// The item ids with a live `build` marker in this repo. `ActiveRun.item` is `Some` only for build
/// runs, so this is exactly the set of in-flight builds here. An unreadable registry is an error,
/// never an empty list: this set is the in-flight guard for `--discard` and `--resume`, and an
/// inconclusive read must not report all-clear and let a live build be pulled or double-driven.
fn live_build_items(repo_record: &str) -> anyhow::Result<Vec<String>> {
    let registry = crate::runs::active()
        .context("could not read the run registry to check for in-flight builds")?;
    Ok(registry
        .runs
        .into_iter()
        .filter(|r| r.command == "build" && r.repo == repo_record)
        .filter_map(|r| r.item)
        .collect())
}

/// Enumerate the per-item worktree subdirectories under this repo's worktrees home. The subdir name
/// is the item id; a name that is not a usable id is disclosed, never acted on.
fn enumerate_worktrees(root: &Path) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let Some(dir) = crate::build_worktrees_dir(root) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let Some(entries) = crate::read_dir_optional(&dir)
        .with_context(|| format!("reading build worktrees under {}", dir.display()))?
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let (mut items, mut unreadable): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for entry in entries {
        let entry = entry
            .with_context(|| format!("reading a build worktree entry in {}", dir.display()))?;
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        match entry.file_name().into_string() {
            Ok(name) if valid_item_id(&name) => items.push(name),
            Ok(name) => unreadable.push(name),
            Err(os) => unreadable.push(os.to_string_lossy().into_owned()),
        }
    }
    items.sort();
    unreadable.sort();
    Ok((items, unreadable))
}

/// The build-grade item-id bar (matches `commands::run_build`): a plain identifier that is safe as a
/// branch, a path component, and a Claude permission-rule specifier at once.
pub(crate) fn valid_item_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains("..")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn run(args: &BuildsArgs, global: &GlobalArgs) -> anyhow::Result<std::process::ExitCode> {
    let root = super::resolve_root(&args.path)?;
    let (_default_branch, default_head) =
        crate::synth::default_branch_base(&root).map_err(|e| {
            anyhow::anyhow!("could not resolve the default branch to derive build state: {e}")
        })?;
    let repo_record = crate::log::repo_record_string(&root);
    let live_items = live_build_items(&repo_record)?;
    let (records, _skipped) = crate::log::records_newest_first()?;
    let s = StateInputs {
        default_head: &default_head,
        repo_record: &repo_record,
        live_items: &live_items,
        records: &records,
    };

    if let Some(item) = &args.resume {
        return resume(&root, item, &s, args, global);
    }
    if let Some(item) = &args.discard {
        discard(&root, item, &s, global)?;
        return Ok(std::process::ExitCode::SUCCESS);
    }
    if args.clean {
        clean(&root, &s, global)?;
        return Ok(std::process::ExitCode::SUCCESS);
    }

    let (items, unreadable) = enumerate_worktrees(&root)?;
    let mut builds = Vec::with_capacity(items.len());
    for item in items {
        let (state, commit) = derive_state(&root, &item, &s)?;
        // Effectively infallible here (the items came from a resolved worktrees dir), so the None
        // is asserted as an error like the sibling sites — never papered over with an empty path.
        let worktree = crate::build_worktree_path(&root, &item)
            .ok_or_else(|| {
                anyhow::anyhow!("could not resolve a worktree path (home directory unknown)")
            })?
            .display()
            .to_string();
        builds.push(BuildEntry {
            branch: build_branch(&item),
            item,
            state,
            worktree,
            commit,
        });
    }

    let report = BuildsReport { builds, unreadable };
    let human = human_report(&report);
    crate::output::emit(&report, &human, global.json)?;
    Ok(std::process::ExitCode::SUCCESS)
}

/// `--resume <item>`: relaunch an *interrupted* build against its worktree's current on-disk state —
/// continuing the interrupted work, never restarting from base. Every other state is refused in its
/// own terms: a live run is never double-driven, and a merged or pending-review build has nothing to
/// resume — discard or review it instead. The relaunch is `arc run build`'s own launcher in resume
/// mode, so shaping, preview, boundary, and logging are the one shared path.
fn resume(
    root: &Path,
    item: &str,
    s: &StateInputs,
    args: &BuildsArgs,
    global: &GlobalArgs,
) -> anyhow::Result<std::process::ExitCode> {
    let _ = item_worktree(root, item)?;
    let (state, _) = derive_state(root, item, s)?;
    match state {
        BuildState::Interrupted => {}
        BuildState::InFlight => anyhow::bail!(
            "the build for `{item}` is in flight — a running agent is never double-driven"
        ),
        BuildState::Merged => anyhow::bail!(
            "the build for `{item}` is merged — nothing to resume; `arc builds --clean` retires it"
        ),
        BuildState::PendingReview => anyhow::bail!(
            "the build for `{item}` is pending review — nothing to resume; review it, or discard it to rebuild"
        ),
    }
    let build_args = crate::cli::BuildArgs {
        item: item.to_owned(),
        path: args.path.clone(),
        model: args.model.clone(),
        backend: args.backend.clone(),
        dry_run: args.dry_run,
        max_budget_usd: args.max_budget_usd,
        ambient_memory: false,
        ruleset: None,
        rules: None,
    };
    super::run_build_mode(&build_args, global, true)
}

/// Resolve the one worktree for `item`, refusing an unusable id or a missing worktree — shared
/// entry validation for the per-item actions.
fn item_worktree(root: &Path, item: &str) -> anyhow::Result<std::path::PathBuf> {
    anyhow::ensure!(
        valid_item_id(item),
        "`{item}` is not a usable build item id — use a plain id (letters, digits, '-', '_', '.')"
    );
    let worktree = crate::build_worktree_path(root, item).ok_or_else(|| {
        anyhow::anyhow!("could not resolve a worktree path (home directory unknown)")
    })?;
    anyhow::ensure!(
        crate::try_is_dir(&worktree)?,
        "no build worktree for `{item}` — `arc builds` lists the ones that exist"
    );
    Ok(worktree)
}

/// `--discard <item>`: remove the item's build worktree and branch. Refused while the build is in
/// flight — a worktree a running agent is writing into is never pulled out from under it; any other
/// state is the operator's to discard.
fn discard(root: &Path, item: &str, s: &StateInputs, global: &GlobalArgs) -> anyhow::Result<()> {
    let worktree = item_worktree(root, item)?;
    let (state, _) = derive_state(root, item, s)?;
    anyhow::ensure!(
        state != BuildState::InFlight,
        "the build for `{item}` is in flight — a running agent owns that worktree; wait for it or kill the run first"
    );
    crate::synth::remove_build_worktree(root, &build_branch(item), &worktree)
        .map_err(|e| anyhow::anyhow!("could not discard the build for `{item}`: {e}"))?;
    #[derive(Serialize)]
    struct Discarded<'a> {
        discarded: &'a str,
        state: BuildState,
        branch: String,
        worktree: String,
    }
    let payload = Discarded {
        discarded: item,
        state,
        branch: build_branch(item),
        worktree: worktree.display().to_string(),
    };
    let human = format!("discarded {item} ({})\n", state.label());
    crate::output::emit(&payload, &human, global.json)
}

/// `--clean`: remove every *merged* build worktree and resolve its item — the merged build is the
/// one whose change has landed, so the worktree is done serving and the agenda entry it implemented
/// closes with the trail (the latest run's commit in the resolution note).
fn clean(root: &Path, s: &StateInputs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (items, _unreadable) = enumerate_worktrees(root)?;
    let mut cleaned = Vec::new();
    for item in items {
        let (state, commit) = derive_state(root, &item, s)?;
        if state != BuildState::Merged {
            continue;
        }
        let worktree = crate::build_worktree_path(root, &item).ok_or_else(|| {
            anyhow::anyhow!("could not resolve a worktree path (home directory unknown)")
        })?;
        crate::synth::remove_build_worktree(root, &build_branch(&item), &worktree)
            .map_err(|e| anyhow::anyhow!("could not clean the merged build for `{item}`: {e}"))?;
        let run_id = latest_build_run(s.records, s.repo_record, &item).and_then(|l| l.run_id);
        let resolution =
            super::items::resolve_item(root, &item, run_id.as_deref(), commit.as_deref())?;
        cleaned.push(CleanedBuild {
            item,
            commit,
            resolution,
        });
    }
    #[derive(Serialize)]
    struct CleanReport {
        cleaned: Vec<CleanedBuild>,
    }
    let human = if cleaned.is_empty() {
        "no merged builds to clean\n".to_owned()
    } else {
        use std::fmt::Write as _;
        let mut out = String::new();
        for c in &cleaned {
            let commit = c.commit.as_deref().unwrap_or("unknown");
            let _ = writeln!(out, "cleaned {} @ {commit} — {}", c.item, c.resolution);
        }
        out
    };
    crate::output::emit(&CleanReport { cleaned }, &human, global.json)
}

#[derive(Serialize)]
struct CleanedBuild {
    item: String,
    /// The latest run's committed sha when the record carries one — `None` disclosed as unknown,
    /// never fabricated.
    commit: Option<String>,
    /// What happened to the agenda entry: resolved (moved to `resolved/`), or why not.
    resolution: String,
}

fn human_report(report: &BuildsReport) -> String {
    use std::fmt::Write as _;
    /// State-label column width: the longest [`BuildState::label`] ("pending review", 14) plus a
    /// two-space gutter, so the item column stays aligned across every state.
    const STATE_WIDTH: usize = 16;
    let mut out = String::new();
    if report.builds.is_empty() {
        out.push_str("no build worktrees\n");
    } else {
        for b in &report.builds {
            let commit = b
                .commit
                .as_deref()
                .map(|c| format!(" @ {c}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "{:<STATE_WIDTH$} {}{}",
                b.state.label(),
                b.item,
                commit
            );
        }
    }
    for name in &report.unreadable {
        let _ = writeln!(out, "(unreadable worktree name: {name})");
    }
    out
}
