//! Active-run registry: each in-flight synthesis writes its own marker to
//! `~/.arc/runs/<pid>-<index>.json`, updates it as the run streams, and removes it on completion (via
//! the returned guard). One file per run — so a `--runs N` fan-out is N independent files, each
//! written only by the thread that owns it: no shared state, no concurrent writes, no locking. This
//! is the live, ephemeral view; the durable record of every completed run is the log in `log.rs`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// One in-flight run, as recorded in the registry: static identity plus live, streamed progress.
#[derive(Serialize, Deserialize)]
pub struct ActiveRun {
    pub pid: u32,
    /// Distinguishes the concurrent runs of a `--runs N` fan-out within one process.
    pub index: usize,
    pub command: String,
    pub repo: String,
    pub model: String,
    pub started_at: u64,
    /// Live progress, updated as the run streams. Every field is written at registration, so there
    /// are no serde defaults: a marker that can't be read in full surfaces as unreadable rather
    /// than as fabricated zeros.
    pub turns: u64,
    pub tool_calls: u64,
    /// Characters — not tokens — because the exact token count arrives only at message end;
    /// characters are the continuous live signal, and the billed token count lands in the final
    /// run report.
    pub output_chars: u64,
}

impl ActiveRun {
    /// Seconds this run has been in flight, formatted for display (`"{n}s"`) — single-sourced so
    /// `arc status` and the TUI's live view can't drift in how they render a run's age.
    pub fn age_display(&self, now: u64) -> String {
        format!("{}s", now.saturating_sub(self.started_at))
    }
}

/// The registry directory, `~/.arc/runs/` (`None` if the home directory is unknown).
fn dir() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("runs"))
}

/// A registered in-flight run: owns its marker and live stats, written only by the single thread that
/// runs it. Removes the marker on drop — success, error, or unwind.
pub struct Active {
    marker: PathBuf,
    run: ActiveRun,
    /// Set once the first marker rewrite fails, so a persistent failure (the registry directory
    /// removed mid-run, a full disk) is surfaced once rather than on every streamed update — the
    /// failure stays visible without flooding the live output.
    warned: bool,
}

impl Active {
    /// Add streamed output characters to the live tally and rewrite the marker (why characters:
    /// see [`ActiveRun::output_chars`]).
    pub fn record_text(&mut self, chars: u64) {
        self.run.output_chars += chars;
        self.write();
    }

    /// Mark one completed turn (and any tool calls it made) and rewrite the marker.
    pub fn record_turn(&mut self, tool_calls: u64) {
        self.run.turns += 1;
        self.run.tool_calls += tool_calls;
        self.write();
    }

    /// Rewrite the marker. `register` already proved the directory writable with the same write, so a
    /// later failure is genuinely exceptional; it can't abort the run it's only observing, so the run
    /// proceeds — but the failure is surfaced (warned once, not on every streamed update), never
    /// dropped silently, the way logging and registration warn. The durable record is the
    /// completed-run log, not this marker.
    fn write(&mut self) {
        if let Err(e) = self.try_write()
            && !self.warned
        {
            self.warned = true;
            eprintln!(
                "arclite: `arc status` progress for this run may be stale (couldn't update its registry marker: {e})"
            );
        }
    }

    fn try_write(&self) -> std::io::Result<()> {
        let json = serde_json::to_string(&self.run).expect("an ActiveRun serializes");
        std::fs::write(&self.marker, json)
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        // Best-effort cleanup, but not silent (the warn-then-proceed standard `write` uses): a failed
        // removal leaves a stale marker that makes `arc status` over-report this run.
        if let Err(e) = std::fs::remove_file(&self.marker) {
            eprintln!(
                "arclite: couldn't remove this run's registry marker {} ({e}); it may linger in `arc status`",
                self.marker.display()
            );
        }
    }
}

/// Record an in-flight run; the returned guard clears its marker on drop. Best-effort: returns
/// `None` (the run proceeds untracked) if the registry can't be written, never failing the run.
#[must_use]
pub fn register(command: &str, repo: &Path, model: &str, index: usize) -> Option<Active> {
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let marker = dir.join(format!("{}-{index}.json", std::process::id()));
    let active = Active {
        marker,
        run: ActiveRun {
            pid: std::process::id(),
            index,
            command: command.to_owned(),
            repo: repo.display().to_string(),
            model: model.to_owned(),
            started_at: crate::log::now_secs(),
            turns: 0,
            tool_calls: 0,
            output_chars: 0,
        },
        warned: false,
    };
    active.try_write().ok()?;
    Some(active)
}

/// The registry as one read sees it: the live runs, entries that couldn't be read or parsed, and
/// the stale markers this read pruned (hard-killed runs' corpses — see [`active`]).
pub struct Registry {
    pub runs: Vec<ActiveRun>,
    pub unreadable: Vec<PathBuf>,
    pub pruned: Vec<PathBuf>,
}

/// Whether process `pid` is alive, probed with the platform's own tool. Unix: `kill -0`, judged by
/// exit code alone — no output parsing, so locales can't skew it; a nonzero exit means no such
/// process *or* a recycled pid now owned by another user, and either way it is not this user's
/// in-flight arc run. Windows: `tasklist` filtered to the pid, matched on the pid appearing as its
/// own whitespace-separated token (the no-match notice carries no bare pid; token match so 123
/// can't match 1234). `None` when the probe itself couldn't run — can't-tell, which callers must
/// treat as alive: wrongly keeping a dead run's marker over-reports it, wrongly pruning a live
/// run's hides real in-flight spend, so an inconclusive probe must not green-light deletion.
fn process_alive(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        let status = crate::ai::command("kill")
            .ok()?
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        Some(status.success())
    }
    #[cfg(windows)]
    {
        let output = crate::ai::command("tasklist")
            .ok()?
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Some(text.split_whitespace().any(|tok| tok == pid.to_string()))
    }
}

/// The runs currently recorded in the registry, for `arc status`, plus any `.json` entries that
/// couldn't be read or parsed (returned, not dropped, so `arc status` surfaces them). A missing
/// registry is not an error — nothing is in flight; a genuine read failure (of the directory or an
/// entry) is — [`crate::read_optional`]'s absent-vs-failed distinction, applied to a directory
/// listing. A marker normally clears on exit; a hard-killed process leaves a stale one (`Drop`
/// never ran), so each marker's pid is probed and a confirmed-dead one is pruned — removed
/// best-effort (a failed removal just re-prunes on the next read) and disclosed in `pruned`,
/// never reported as an active run. An inconclusive probe keeps the marker (see [`process_alive`]).
pub fn active() -> anyhow::Result<Registry> {
    let mut registry = Registry {
        runs: Vec::new(),
        unreadable: Vec::new(),
        pruned: Vec::new(),
    };
    let Some(dir) = dir() else {
        return Ok(registry);
    };
    let Some(entries) = crate::read_dir_optional(&dir)
        .with_context(|| format!("cannot read the run registry {}", dir.display()))?
    else {
        return Ok(registry);
    };
    for entry in entries {
        let path = entry
            .with_context(|| format!("cannot read an entry in the run registry {}", dir.display()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // Read through the shared optional-read, holding the absent-vs-unreadable distinction: a
        // marker that raced away (absent — a run finished and cleared it mid-listing) is benign and
        // skipped, while one present-but-unreadable or unparseable is surfaced as unreadable, never
        // propagated (one bad marker shouldn't fail `arc status`).
        match crate::read_optional(&path) {
            Ok(Some(text)) => match serde_json::from_str::<ActiveRun>(&text) {
                Ok(run) => {
                    if process_alive(run.pid) == Some(false) {
                        let _ = std::fs::remove_file(&path);
                        registry.pruned.push(path);
                    } else {
                        registry.runs.push(run);
                    }
                }
                Err(_) => registry.unreadable.push(path), // present but unparseable — a real problem
            },
            Ok(None) => {} // the marker raced away as the run finished — benign, not a failure
            Err(_) => registry.unreadable.push(path),
        }
    }
    Ok(registry)
}

/// Human phrasing for pruned stale markers — shared by `arc status` and the TUI, like
/// [`unreadable_entries`].
pub fn pruned_entries(count: usize) -> String {
    format!(
        "pruned {count} stale marker{} (process gone — a hard-killed run)",
        if count == 1 { "" } else { "s" }
    )
}

/// Human phrasing for a count of registry entries that couldn't be read or parsed — the singular/plural
/// wording shared by `arc status` and the TUI, so the two can't drift.
pub fn unreadable_entries(count: usize) -> String {
    format!(
        "{count} unreadable registry entr{}",
        if count == 1 { "y" } else { "ies" }
    )
}
