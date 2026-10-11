//! Device discovery — the repositories on this machine and the agent activity around them, known
//! by discovery rather than declaration, read from indexes that already exist: the agent CLIs' own
//! session stores and arc's run ledger. Each session file contributes its head record's metadata
//! (`cwd`, and codex's `source` kind) and its mtime — a head read suffices because both stores
//! put the session metadata first (see [`HEAD_SCAN_LINES`]).
//!
//! The session stores are external surfaces with no layout contract (observed on this machine
//! 2026-08-23, source kinds 2026-09-25; every read here fails soft per store, disclosed in the
//! result's notes, so a store that moves or changes shape degrades the map rather than erroring
//! the command):
//! - Claude Code: `~/.claude/projects/<encoded-path>/` — one directory per project with session
//!   `.jsonl` files inside. The directory name encodes the repo path lossily (`/` → `-`), so the
//!   path is taken from a session head's `cwd` where readable, with the name decode as a
//!   disclosed fallback.
//! - codex: `~/.codex/sessions/**/rollout-*.jsonl` — loose files and dated subdirectories both
//!   observed; the repo lives inside the record (a head line's `cwd`), not in the filename. This
//!   is the codex CLI's own session store (`codex-rs/rollout` in the open source, whose meta line
//!   carries `cwd` and `source`; verified 2026-09-25). The meta's `source` separates headless
//!   `exec` runs — arc's own codex synthesis lands there — from interactive sessions, so the map
//!   counts them apart instead of reporting arc's own residue as agent sessions. `~/.codex/sqlite/`
//!   also exists but is deliberately not read: its thread catalog (`codex-dev.db`) is written by
//!   the ChatGPT desktop app, not the CLI (schema closed-source; `lsof` + binary-strings check,
//!   2026-09-25), and the CLI's own state DBs there were months stale on this machine while
//!   rollouts stayed current.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

use serde::Serialize;

/// The activity-recency window under which a session reads as in flight: a store file written
/// within the last two minutes. An mtime heuristic — displayed as [`ACTIVE_LABEL`], deliberately a
/// recency claim and never an asserted live connection (only arc's own run markers prove liveness,
/// and only for arc runs).
pub const ACTIVE_WINDOW_SECS: u64 = 120;

/// How activity inside [`ACTIVE_WINDOW_SECS`] is labeled — one wording for the CLI and the TUI.
pub const ACTIVE_LABEL: &str = "active now";

/// How many head lines of a session file are scanned for its metadata before giving up: codex
/// writes the session meta as the rollout's first record by design (`codex-rs/rollout`, verified
/// 2026-09-25), and Claude Code's store carries `cwd` within the first records (observed); the
/// allowance covers format variants, and past it there is no metadata left to find — scanning
/// deeper would only cost. The default, overridable per [`head_scan_bounds`].
const DEFAULT_HEAD_SCAN_LINES: usize = 5;

/// A head line longer than this is not metadata; the scan stops rather than buffering content.
/// The default, overridable per [`head_scan_bounds`].
const DEFAULT_HEAD_SCAN_MAX_BYTES: u64 = 64 * 1024;

/// Env knobs letting the caller widen or disable the head-scan bounds when the defaults would omit
/// a repo whose metadata sits deeper: `ARC_DISCOVERY_HEAD_SCAN_LINES` and
/// `ARC_DISCOVERY_HEAD_SCAN_BYTES`, each `0` meaning unbounded (scan the whole file). Unset uses
/// the defaults above. Read once per process.
fn head_scan_bounds() -> (usize, u64) {
    use std::sync::LazyLock;
    static BOUNDS: LazyLock<(usize, u64)> = LazyLock::new(|| {
        let lines = match std::env::var("ARC_DISCOVERY_HEAD_SCAN_LINES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            Some(0) => usize::MAX,
            Some(n) => n,
            None => DEFAULT_HEAD_SCAN_LINES,
        };
        let bytes = match std::env::var("ARC_DISCOVERY_HEAD_SCAN_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            Some(0) => u64::MAX,
            Some(n) => n,
            None => DEFAULT_HEAD_SCAN_MAX_BYTES,
        };
        (lines, bytes)
    });
    *BOUNDS
}

/// Warnings for a head-scan knob set to an unreadable value: an env var present but unparseable
/// means the caller *tried* to shape completeness and was ignored — disclosed, never a silent
/// fall-through to the default (the default still applies, per [`head_scan_bounds`]).
fn head_scan_env_warnings() -> Vec<String> {
    let mut w = Vec::new();
    let present_but_unparsed = |key: &str| {
        std::env::var_os(key).is_some_and(|v| {
            // Non-Unicode (to_str == None) or non-numeric both count as unreadable; an empty
            // value is treated as unset, not an error.
            match v.to_str().map(str::trim) {
                None => true,
                Some("") => false,
                Some(s) => s.parse::<u64>().is_err(),
            }
        })
    };
    for key in [
        "ARC_DISCOVERY_HEAD_SCAN_LINES",
        "ARC_DISCOVERY_HEAD_SCAN_BYTES",
    ] {
        if present_but_unparsed(key) {
            w.push(format!(
                "{key} is set to an unreadable value — ignored, the built-in default applies"
            ));
        }
    }
    w
}

/// The external session stores' observed layout (2026-08-23; no layout contract — see the module
/// doc), named here so a store's move or rename is one edit rather than a literal hunt across the
/// readers, and so the shared session-file extension has a single home. Each store's own reader
/// documents how it uses these.
const CLAUDE_STORE_DIR: &str = ".claude";
const CLAUDE_PROJECTS_SUBDIR: &str = "projects";
const CODEX_STORE_DIR: &str = ".codex";
const CODEX_SESSIONS_SUBDIR: &str = "sessions";
const CODEX_ROLLOUT_PREFIX: &str = "rollout-";
/// The session-meta `source` value codex records for a headless `codex exec` run.
const CODEX_EXEC_SOURCE: &str = "exec";
/// The session-file extension both stores use (Claude's per-project files and codex's rollouts).
const SESSION_FILE_EXT: &str = "jsonl";

/// The discovery indexes — what a repo's `last_source` names.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Claude,
    Codex,
    Ledger,
}

impl Source {
    /// The lowercase name every display uses (matching the serialized form).
    fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Ledger => "ledger",
        }
    }
}

/// One repository's discovered activity across the three indexes.
#[derive(Serialize)]
pub struct RepoActivity {
    /// The repo path, in the ledger's record-string form where it came from a `cwd`/record, or the
    /// lossy name decode where no session head was readable (counted in the notes).
    pub repo: String,
    pub claude_sessions: usize,
    pub codex_sessions: usize,
    /// codex rollouts whose head meta says `source: "exec"` — headless `codex exec` runs (arc's
    /// own codex synthesis among them), counted apart so the sessions column stays sessions.
    pub codex_exec_runs: usize,
    pub ledger_runs: usize,
    /// Most recent activity across the sources (unix secs); `None` when only counts are known.
    pub last_activity: Option<u64>,
    /// Which index produced [`Self::last_activity`].
    pub last_source: Option<Source>,
    /// Whether the path no longer exists on disk — the stores remember ephemeral worktrees and
    /// deleted checkouts long after they're gone (the first real run surfaced ~150 of them), so
    /// the default view folds gone paths into a count and `--all` shows them. Discovery still
    /// reports them: the history is real even when the directory isn't.
    pub gone: bool,
    /// Whether this row's path came from the lossy project-dir name decode rather than a session
    /// head's `cwd` — disclosed per row because the decoded form is also the key the existence
    /// check and mute matching run against, so a dash-ambiguous decode can misaddress both.
    pub name_decoded: bool,
}

impl RepoActivity {
    /// The recency cell: [`ACTIVE_LABEL`] inside the window, else the shared coarse age, else `?`.
    /// The window itself prints with the label (`≤{ACTIVE_WINDOW_SECS}s`) so the threshold behind
    /// the classification is disclosed in the output, never a silent constant.
    pub fn recency(&self, now: u64) -> String {
        match self.last_activity {
            Some(ts) if now.saturating_sub(ts) <= ACTIVE_WINDOW_SECS => {
                let source = self.last_source.map_or("?", Source::label);
                format!("{ACTIVE_LABEL} ≤{ACTIVE_WINDOW_SECS}s ({source})")
            }
            Some(ts) => crate::commands::log::age(now.saturating_sub(ts)),
            None => "?".to_owned(),
        }
    }
}

/// The discovered map: repos newest-activity-first, the muted count (the one `muted_repos` lens,
/// applied here as on every default cross-repo surface unless bypassed), and every store's
/// disclosures — absent vs unreadable stores, lossy decodes, heads without a readable repo.
/// Nothing here is a hard error: the map degrades per store and says so.
#[derive(Serialize)]
pub struct Discovery {
    pub repos: Vec<RepoActivity>,
    pub muted: usize,
    /// Repos whose paths no longer exist on disk: counted here always; listed in `repos` only
    /// under `include_all`.
    pub gone: usize,
    pub notes: Vec<String>,
}

/// Build the device map. `include_all` bypasses the default lenses — the mute, and the fold of
/// gone paths (an explicit selection bypasses a default lens, as everywhere).
pub fn discover(include_all: bool) -> Discovery {
    let mut map: BTreeMap<String, RepoActivity> = BTreeMap::new();
    let mut notes = head_scan_env_warnings();
    // Surface the attribution method unconditionally, not only when a file fails to resolve: the
    // head-scan bound shapes what the map can see, and the caller should know it is in effect
    // rather than discover it. Names the bound and the knob that widens or disables it.
    let (max_lines, max_bytes) = head_scan_bounds();
    notes.push(if max_lines == usize::MAX {
        "sessions attributed by scanning each file's head (unbounded; ARC_DISCOVERY_HEAD_SCAN_LINES/_BYTES set the cap)".to_owned()
    } else {
        format!(
            "sessions attributed by scanning each file's head (first {max_lines} line(s), {} KiB; ARC_DISCOVERY_HEAD_SCAN_LINES/_BYTES to widen, 0 to disable)",
            max_bytes / 1024
        )
    });
    claude_store(&mut map, &mut notes);
    codex_store(&mut map, &mut notes);
    ledger(&mut map, &mut notes);
    // Absent and unreadable are different truths here too: a path that can't be *checked* is kept
    // in the view (marking it gone would fold real data out on a failed stat — a suppression must
    // fail toward showing), disclosed in the notes.
    let mut uncheckable = 0usize;
    for r in map.values_mut() {
        r.gone = match crate::try_is_dir(Path::new(&r.repo)) {
            Ok(is_dir) => !is_dir,
            Err(_) => {
                uncheckable += 1;
                false
            }
        };
    }
    if uncheckable > 0 {
        notes.push(format!(
            "{uncheckable} repo path(s) could not be checked for existence — kept in the view"
        ));
    }

    // The one mute lens, from its one home: criteria via `log::mute_criteria` (fail-open on
    // unreadable settings, disclosed) and matching via `log::split_muted_by` — this surface adds
    // only its own bypass (a muted repo absent from the ledger simply mutes nothing there).
    let (mute_list, settings_error) = crate::log::mute_criteria();
    if let Some(e) = settings_error {
        notes.push(format!("settings unreadable — nothing muted: {e}"));
    }
    let mut repos: Vec<RepoActivity> = map.into_values().collect();
    let mut muted = 0usize;
    if !include_all {
        let (kept, dropped) = crate::log::split_muted_by(repos, &mute_list, |r| r.repo.clone());
        repos = kept;
        muted = dropped;
    }
    let gone = if include_all {
        repos.iter().filter(|r| r.gone).count()
    } else {
        let (kept, dropped): (Vec<_>, Vec<_>) = repos.into_iter().partition(|r| !r.gone);
        repos = kept;
        dropped.len()
    };
    repos.sort_by(|a, b| {
        b.last_activity
            .cmp(&a.last_activity)
            .then_with(|| a.repo.cmp(&b.repo))
    });
    Discovery {
        repos,
        muted,
        gone,
        notes,
    }
}

/// The counter one observation accumulates into — an enum, so a count added later must declare
/// its own accumulation arm in [`observe`] at compile time instead of falling into another's
/// catch-all. Distinct from [`Source`] because one index can count more than one thing (the
/// codex store: sessions and exec runs).
#[derive(Clone, Copy)]
enum Counter {
    ClaudeSessions,
    CodexSessions,
    CodexExecRuns,
    LedgerRuns,
}

impl Counter {
    /// The index the observation came from — what `last_source` reports.
    fn source(self) -> Source {
        match self {
            Self::ClaudeSessions => Source::Claude,
            Self::CodexSessions | Self::CodexExecRuns => Source::Codex,
            Self::LedgerRuns => Source::Ledger,
        }
    }
}

/// Fold one observation into the map: `count` accumulates into `counter`, and the newest
/// activity across all observations wins the recency cell.
fn observe(
    map: &mut BTreeMap<String, RepoActivity>,
    repo: String,
    counter: Counter,
    count: usize,
    activity: Option<u64>,
    name_decoded: bool,
) {
    let entry = map.entry(repo.clone()).or_insert_with(|| RepoActivity {
        repo,
        claude_sessions: 0,
        codex_sessions: 0,
        codex_exec_runs: 0,
        ledger_runs: 0,
        last_activity: None,
        last_source: None,
        gone: false,
        name_decoded: false,
    });
    entry.name_decoded |= name_decoded;
    match counter {
        Counter::ClaudeSessions => entry.claude_sessions += count,
        Counter::CodexSessions => entry.codex_sessions += count,
        Counter::CodexExecRuns => entry.codex_exec_runs += count,
        Counter::LedgerRuns => entry.ledger_runs += count,
    }
    if activity > entry.last_activity {
        entry.last_activity = activity;
        entry.last_source = Some(counter.source());
    }
}

/// One store's read failure vs absence, told apart in the notes (absent is a fact about the
/// machine — that CLI isn't in use — while unreadable is a failed check).
fn store_dir(notes: &mut Vec<String>, label: &str, dir: &Path) -> Option<std::fs::ReadDir> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!("{label}: store not present"));
            None
        }
        Err(e) => {
            notes.push(format!("{label}: store unreadable — {e}"));
            None
        }
    }
}

/// A listing's deliverable entries, the dirents the iteration itself failed to yield counted into
/// `lost` — a failed dirent is one more store degradation to disclose, never a silent skip.
fn deliverable(
    entries: std::fs::ReadDir,
    lost: &mut usize,
) -> impl Iterator<Item = std::fs::DirEntry> + '_ {
    entries.filter_map(move |e| match e {
        Ok(e) => Some(e),
        Err(_) => {
            *lost += 1;
            None
        }
    })
}

/// One store's count→disclosure note (`{label}: {n} {what} — skipped, the map undercounts`) — the
/// single wording home for the per-store degradation counts, so the phrasing can't rot per store
/// the way copy-pasted notes would; a zero count says nothing.
fn skipped_note(notes: &mut Vec<String>, label: &str, n: usize, what: &str) {
    if n > 0 {
        notes.push(format!(
            "{label}: {n} {what} — skipped, the map undercounts"
        ));
    }
}

/// Read a session file's head metadata: the `cwd`, plus the `source` kind where the same record
/// carries one (codex's session meta does; Claude Code's store and old codex formats don't).
/// Bounded by [`HEAD_SCAN_LINES`].
/// Tri-state so the caller can tell an *unreadable* head (a real degradation to disclose) from a
/// *readable file that simply has no `cwd`* (an old format, genuinely unattributable): `Err` is
/// the open/read failure, `Ok(None)` the readable-but-no-metadata case, `Ok(Some)` a hit.
fn head_meta(path: &Path) -> std::io::Result<Option<(String, Option<serde_json::Value>)>> {
    use std::io::Read;
    let (max_lines, max_bytes) = head_scan_bounds();
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file.take(max_bytes));
    let mut line = String::new();
    for _ in 0..max_lines {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            // The fields sit at the top level (Claude Code) or under the head record's payload
            // (codex's session-meta shape) — both observed; scan both, assert neither.
            if let Some(cwd) = v.get("cwd").and_then(serde_json::Value::as_str) {
                return Ok(Some((cwd.to_owned(), v.get("source").cloned())));
            }
            if let Some(cwd) = v
                .pointer("/payload/cwd")
                .and_then(serde_json::Value::as_str)
            {
                return Ok(Some((
                    cwd.to_owned(),
                    v.pointer("/payload/source").cloned(),
                )));
            }
        }
    }
    Ok(None)
}

/// Claude Code's per-project store: one directory per project, sessions as `.jsonl` files.
fn claude_store(map: &mut BTreeMap<String, RepoActivity>, notes: &mut Vec<String>) {
    let Some(dir) = dirs::home_dir().map(|h| h.join(CLAUDE_STORE_DIR).join(CLAUDE_PROJECTS_SUBDIR))
    else {
        return;
    };
    let Some(entries) = store_dir(notes, "claude sessions", &dir) else {
        return;
    };
    let mut decoded = 0usize;
    let mut unreadable = 0usize;
    let mut unreadable_mtime = 0usize;
    let mut unreadable_head = 0usize;
    let mut uncheckable = 0usize;
    let mut lost = 0usize;
    let mut lost_files = 0usize;
    for entry in deliverable(entries, &mut lost) {
        let project = entry.path();
        // A bare `.is_dir()` would map a stat failure to "not a directory" and skip it silently;
        // `try_is_dir` keeps the present-but-unstattable case distinct and disclosed (a stray
        // non-directory entry is a legitimate skip; an unstattable one undercounts the map).
        match crate::try_is_dir(&project) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => {
                uncheckable += 1;
                continue;
            }
        }
        let mut sessions = 0usize;
        let mut newest: Option<(u64, std::path::PathBuf)> = None;
        // A project dir that exists but can't be read is counted and disclosed — skipping it
        // silently would collapse unreadable into absent, undercounting with no signal.
        let files = match std::fs::read_dir(&project) {
            Ok(files) => files,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        for f in deliverable(files, &mut lost_files) {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some(SESSION_FILE_EXT) {
                continue;
            }
            sessions += 1;
            // The file exists (it came from the listing), so a `None` mtime is a read failure to
            // disclose, not a benign absence — count it rather than letting it look like no activity.
            match mtime_secs(&p) {
                Some(m) => {
                    if newest.as_ref().is_none_or(|(n, _)| m > *n) {
                        newest = Some((m, p));
                    }
                }
                None => unreadable_mtime += 1,
            }
        }
        if sessions == 0 {
            continue; // an empty project dir names no activity to map
        }
        // `newest` can be absent even with sessions counted — a file raced away between listing
        // and stat, or an unreadable mtime — so the sessions still map (activity unknown, the
        // repo name from the decode fallback) rather than panicking on an assumed invariant.
        let (activity, repo, name_decoded) = match newest {
            Some((mtime, path)) => {
                // An unreadable head is a degradation to disclose; a readable head with no `cwd`
                // is a clean decode fallback — both end at the name decode, but only the first
                // is counted as a read failure.
                let cwd = match head_meta(&path) {
                    Ok(Some((cwd, _))) => Some(cwd),
                    Ok(None) => None,
                    Err(_) => {
                        unreadable_head += 1;
                        None
                    }
                };
                match cwd {
                    Some(cwd) => (Some(mtime), cwd, false),
                    None => {
                        decoded += 1;
                        (
                            Some(mtime),
                            decode_project_dir(&entry.file_name().to_string_lossy()),
                            true,
                        )
                    }
                }
            }
            None => (
                None,
                {
                    decoded += 1;
                    decode_project_dir(&entry.file_name().to_string_lossy())
                },
                true,
            ),
        };
        observe(
            map,
            repo,
            Counter::ClaudeSessions,
            sessions,
            activity,
            name_decoded,
        );
    }
    if decoded > 0 {
        notes.push(format!(
            "claude sessions: {decoded} project(s) resolved by name decoding (lossy — a dash in a \
             real path component reads as a separator)"
        ));
    }
    skipped_note(
        notes,
        "claude sessions",
        unreadable,
        "project dir(s) unreadable",
    );
    if unreadable_mtime > 0 {
        notes.push(format!(
            "claude sessions: {unreadable_mtime} session file(s) counted but with an unreadable \
             mtime — their activity is unknown, not absent"
        ));
    }
    if unreadable_head > 0 {
        notes.push(format!(
            "claude sessions: {unreadable_head} project(s) whose newest session head was unreadable \
             — repo taken from the name decode, a read failure, not a clean no-metadata decode"
        ));
    }
    skipped_note(
        notes,
        "claude sessions",
        lost + lost_files,
        "directory entry(ies) unlistable",
    );
    skipped_note(
        notes,
        "claude sessions",
        uncheckable,
        "entry(ies) couldn't be checked",
    );
}

/// The lossy inverse of Claude Code's project-dir encoding (`/` → `-`).
fn decode_project_dir(name: &str) -> String {
    name.strip_prefix('-').map_or_else(
        || name.to_owned(),
        |rest| format!("/{}", rest.replace('-', "/")),
    )
}

/// codex's session store: `rollout-*.jsonl`, loose and under dated subdirectories.
fn codex_store(map: &mut BTreeMap<String, RepoActivity>, notes: &mut Vec<String>) {
    let Some(dir) = dirs::home_dir().map(|h| h.join(CODEX_STORE_DIR).join(CODEX_SESSIONS_SUBDIR))
    else {
        return;
    };
    if store_dir(notes, "codex sessions", &dir).is_none() {
        return;
    }
    let mut unresolved = 0usize;
    let mut unlabeled = 0usize;
    let mut unreadable = 0usize;
    let mut unreadable_mtime = 0usize;
    let mut unreadable_head = 0usize;
    let mut uncheckable = 0usize;
    let mut lost = 0usize;
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        // An unreadable subdirectory is counted and disclosed, never conflated with empty.
        let entries = match std::fs::read_dir(&d) {
            Ok(entries) => entries,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        for entry in deliverable(entries, &mut lost) {
            let p = entry.path();
            // `try_is_dir`, not a bare `.is_dir()`: a stat failure must not silently demote an
            // unreadable subdirectory to "a file" (which the rollout check would then skip
            // undisclosed) — the same distinction the `read_dir` arm above draws.
            match crate::try_is_dir(&p) {
                Ok(true) => {
                    stack.push(p);
                    continue;
                }
                Ok(false) => {}
                Err(_) => {
                    uncheckable += 1;
                    continue;
                }
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_rollout = name.starts_with(CODEX_ROLLOUT_PREFIX)
                && p.extension().and_then(|e| e.to_str()) == Some(SESSION_FILE_EXT);
            if !is_rollout {
                continue;
            }
            match head_meta(&p) {
                Ok(Some((repo, source))) => {
                    // Classified by codex's own kind label: `exec` is a headless `codex exec`
                    // run (arc's own synthesis lands there); any other present value — `cli`,
                    // `vscode`, or a structured subagent record — is a session/thread. A file
                    // predating the label can't be told apart, so it counts as a session,
                    // disclosed below rather than silently bucketed.
                    let counter = match &source {
                        Some(serde_json::Value::String(s)) if s == CODEX_EXEC_SOURCE => {
                            Counter::CodexExecRuns
                        }
                        Some(_) => Counter::CodexSessions,
                        None => {
                            unlabeled += 1;
                            Counter::CodexSessions
                        }
                    };
                    let activity = mtime_secs(&p);
                    if activity.is_none() {
                        unreadable_mtime += 1;
                    }
                    observe(map, repo, counter, 1, activity, false);
                }
                // Readable but no repo in the head (an old format) — genuinely unattributable.
                Ok(None) => unresolved += 1,
                // The head couldn't be read at all — a degradation, counted apart from the clean
                // no-metadata case so an unreadable rollout isn't filed as "simply has no repo."
                Err(_) => unreadable_head += 1,
            }
        }
    }
    if unresolved > 0 {
        let (max_lines, max_bytes) = head_scan_bounds();
        let bound = if max_lines == usize::MAX {
            "scan unbounded".to_owned()
        } else {
            format!(
                "scan bounded to the first {max_lines} line(s), {} KiB",
                max_bytes / 1024
            )
        };
        notes.push(format!(
            "codex sessions: {unresolved} session file(s) without a readable repo in the head \
             ({bound}; widen with ARC_DISCOVERY_HEAD_SCAN_LINES/_BYTES) — counted nowhere rather \
             than guessed"
        ));
    }
    if unlabeled > 0 {
        notes.push(format!(
            "codex sessions: {unlabeled} file(s) predate the source label — counted as sessions"
        ));
    }
    if unreadable_head > 0 {
        notes.push(format!(
            "codex sessions: {unreadable_head} rollout(s) whose head couldn't be read — a read \
             failure, counted apart from the files that are simply without a repo in the head"
        ));
    }
    if unreadable_mtime > 0 {
        notes.push(format!(
            "codex sessions: {unreadable_mtime} rollout(s) counted but with an unreadable mtime \
             — their activity is unknown, not absent"
        ));
    }
    skipped_note(
        notes,
        "codex sessions",
        unreadable,
        "subdirectory(ies) unreadable",
    );
    skipped_note(
        notes,
        "codex sessions",
        lost,
        "directory entry(ies) unlistable",
    );
    skipped_note(
        notes,
        "codex sessions",
        uncheckable,
        "entry(ies) couldn't be checked",
    );
}

/// arc's own ledger: run counts and last-run recency per recorded repo.
fn ledger(map: &mut BTreeMap<String, RepoActivity>, notes: &mut Vec<String>) {
    match crate::log::records_newest_first() {
        Ok((records, unparsed)) => {
            if unparsed > 0 {
                notes.push(format!("ledger: {}", crate::log::unparsed_note(unparsed)));
            }
            for r in &records {
                let Some(repo) = crate::log::record_repo(r) else {
                    continue;
                };
                let ts = r.get("ts").and_then(serde_json::Value::as_u64);
                observe(map, repo.to_owned(), Counter::LedgerRuns, 1, ts, false);
            }
        }
        Err(e) => notes.push(format!("ledger unreadable: {e:#}")),
    }
}

/// A file's mtime as unix seconds, `None` where the platform or filesystem won't say. For a file
/// known to exist (one taken from a directory listing), a `None` is a read failure the caller
/// discloses as activity-unknown, never silently folded into activity-absent.
fn mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
