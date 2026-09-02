//! Device discovery — the repositories on this machine and the agent activity around them, known
//! by discovery rather than declaration, read from indexes that already exist: the agent CLIs' own
//! session stores and arc's run ledger. Read-only enumeration of names and file metadata — no
//! transcript content is read at this layer (a session file contributes the `cwd` in its head and
//! its mtime, nothing else).
//!
//! The session stores are external surfaces with no layout contract (observed on this machine
//! 2026-08-23; every read here fails soft per store, disclosed in the result's notes, so a store
//! that moves or changes shape degrades the map rather than erroring the command):
//! - Claude Code: `~/.claude/projects/<encoded-path>/` — one directory per project with session
//!   `.jsonl` files inside. The directory name encodes the repo path lossily (`/` → `-`), so the
//!   path is taken from a session head's `cwd` where readable, with the name decode as a
//!   disclosed fallback.
//! - codex: `~/.codex/sessions/**/rollout-*.jsonl` — loose files and dated subdirectories both
//!   observed; the repo lives inside the record (a head line's `cwd`), not in the filename.

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

/// How many head lines of a session file are scanned for its `cwd` before giving up: the field
/// sits in the first record(s) in both observed stores, and a deeper scan would start reading
/// transcript body — exactly what this layer must not do.
const HEAD_SCAN_LINES: usize = 5;

/// A head line longer than this is not metadata; the scan stops rather than buffering content.
const HEAD_SCAN_MAX_BYTES: u64 = 64 * 1024;

/// One repository's discovered activity across the three indexes.
#[derive(Serialize)]
pub struct RepoActivity {
    /// The repo path, in the ledger's record-string form where it came from a `cwd`/record, or the
    /// lossy name decode where no session head was readable (counted in the notes).
    pub repo: String,
    pub claude_sessions: usize,
    pub codex_sessions: usize,
    pub ledger_runs: usize,
    /// Most recent activity across the sources (unix secs); `None` when only counts are known.
    pub last_activity: Option<u64>,
    /// Which index produced [`Self::last_activity`].
    pub last_source: Option<&'static str>,
    /// Whether the path no longer exists on disk — the stores remember ephemeral worktrees and
    /// deleted checkouts long after they're gone (the first real run surfaced ~150 of them), so
    /// the default view folds gone paths into a count and `--all` shows them. Discovery still
    /// reports them: the history is real even when the directory isn't.
    pub gone: bool,
}

impl RepoActivity {
    /// The recency cell: [`ACTIVE_LABEL`] inside the window, else the shared coarse age, else `?`.
    pub fn recency(&self, now: u64) -> String {
        match self.last_activity {
            Some(ts) if now.saturating_sub(ts) <= ACTIVE_WINDOW_SECS => {
                let source = self.last_source.unwrap_or("?");
                format!("{ACTIVE_LABEL} ({source})")
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
    let mut notes = Vec::new();
    claude_store(&mut map, &mut notes);
    codex_store(&mut map, &mut notes);
    ledger(&mut map, &mut notes);
    for r in map.values_mut() {
        r.gone = !Path::new(&r.repo).is_dir();
    }

    // The one mute lens: a mute list is just repo paths, and every default cross-repo surface —
    // ledger views and this map alike — filters its own entries against the same `muted_repos` by
    // exact match (a muted repo absent from the ledger simply mutes nothing there). Fails open:
    // unreadable settings mute nothing, disclosed.
    let mut muted = 0usize;
    let mut repos: Vec<RepoActivity> = match crate::settings::Settings::load(Path::new(".")) {
        Ok(s) if !include_all && !s.muted_repos.is_empty() => {
            let (kept, dropped): (Vec<_>, Vec<_>) = map
                .into_values()
                .partition(|r| !s.muted_repos.contains(&r.repo));
            muted = dropped.len();
            kept
        }
        Ok(_) => map.into_values().collect(),
        Err(e) => {
            notes.push(format!("settings unreadable — nothing muted: {e:#}"));
            map.into_values().collect()
        }
    };
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

/// Fold one source's observation into the map.
fn observe(
    map: &mut BTreeMap<String, RepoActivity>,
    repo: String,
    source: &'static str,
    sessions: usize,
    runs: usize,
    activity: Option<u64>,
) {
    let entry = map.entry(repo.clone()).or_insert_with(|| RepoActivity {
        repo,
        claude_sessions: 0,
        codex_sessions: 0,
        ledger_runs: 0,
        last_activity: None,
        last_source: None,
        gone: false,
    });
    match source {
        "claude" => entry.claude_sessions += sessions,
        "codex" => entry.codex_sessions += sessions,
        _ => entry.ledger_runs += runs,
    }
    if activity > entry.last_activity {
        entry.last_activity = activity;
        entry.last_source = Some(source);
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

/// Scan a session file's head for its `cwd` — metadata only, bounded by [`HEAD_SCAN_LINES`].
fn head_cwd(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file.take(HEAD_SCAN_MAX_BYTES));
    let mut line = String::new();
    for _ in 0..HEAD_SCAN_LINES {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            // The field sits at the top level (Claude Code) or under the head record's payload
            // (codex's session-meta shape) — both observed; scan both, assert neither.
            if let Some(cwd) = v.get("cwd").and_then(serde_json::Value::as_str) {
                return Some(cwd.to_owned());
            }
            if let Some(cwd) = v
                .pointer("/payload/cwd")
                .and_then(serde_json::Value::as_str)
            {
                return Some(cwd.to_owned());
            }
        }
    }
    None
}

/// Claude Code's per-project store: one directory per project, sessions as `.jsonl` files.
fn claude_store(map: &mut BTreeMap<String, RepoActivity>, notes: &mut Vec<String>) {
    let Some(dir) = dirs::home_dir().map(|h| h.join(".claude").join("projects")) else {
        return;
    };
    let Some(entries) = store_dir(notes, "claude sessions", &dir) else {
        return;
    };
    let mut decoded = 0usize;
    for entry in entries.flatten() {
        let project = entry.path();
        if !project.is_dir() {
            continue;
        }
        let mut sessions = 0usize;
        let mut newest: Option<(u64, std::path::PathBuf)> = None;
        if let Ok(files) = std::fs::read_dir(&project) {
            for f in files.flatten() {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                sessions += 1;
                if let Some(m) = mtime_secs(&p)
                    && newest.as_ref().is_none_or(|(n, _)| m > *n)
                {
                    newest = Some((m, p));
                }
            }
        }
        if sessions == 0 {
            continue; // an empty project dir names no activity to map
        }
        let (mtime, newest_path) = newest.expect("sessions > 0 implies a newest file");
        let repo = head_cwd(&newest_path).unwrap_or_else(|| {
            decoded += 1;
            decode_project_dir(&entry.file_name().to_string_lossy())
        });
        observe(map, repo, "claude", sessions, 0, Some(mtime));
    }
    if decoded > 0 {
        notes.push(format!(
            "claude sessions: {decoded} project(s) resolved by name decoding (lossy — a dash in a \
             real path component reads as a separator)"
        ));
    }
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
    let Some(dir) = dirs::home_dir().map(|h| h.join(".codex").join("sessions")) else {
        return;
    };
    if store_dir(notes, "codex sessions", &dir).is_none() {
        return;
    }
    let mut unresolved = 0usize;
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                continue;
            }
            match head_cwd(&p) {
                Some(repo) => observe(map, repo, "codex", 1, 0, mtime_secs(&p)),
                None => unresolved += 1,
            }
        }
    }
    if unresolved > 0 {
        notes.push(format!(
            "codex sessions: {unresolved} session file(s) without a readable repo in the head — \
             counted nowhere rather than guessed"
        ));
    }
}

/// arc's own ledger: run counts and last-run recency per recorded repo.
fn ledger(map: &mut BTreeMap<String, RepoActivity>, notes: &mut Vec<String>) {
    match crate::log::records_newest_first() {
        Ok((records, unparsed)) => {
            if unparsed > 0 {
                notes.push(format!("ledger: {}", crate::log::unparsed_note(unparsed)));
            }
            for r in &records {
                let repo = crate::log::field(r, "repo");
                if repo.is_empty() {
                    continue;
                }
                let ts = r.get("ts").and_then(serde_json::Value::as_u64);
                observe(map, repo, "ledger", 0, 1, ts);
            }
        }
        Err(e) => notes.push(format!("ledger unreadable: {e:#}")),
    }
}

/// A file's mtime as unix seconds, `None` where the platform or filesystem won't say.
fn mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
