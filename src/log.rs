//! Append-only per-run logging to `~/.arc/logs/runs.jsonl` — a record of every AI run (params,
//! context, tokens, cost), plus each run's full result at `~/.arc/logs/results/<id>.json`. On by
//! default; disable via `defaults.logging = false`. A write failure warns but never fails the command.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Current UNIX time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the UNIX epoch")
        .as_secs()
}

/// The arclite logs directory, `~/.arc/logs` — the single source the run log and the result store
/// both build on (`None` only if the home directory can't be determined).
fn logs_dir() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("logs"))
}

/// Path of the run log, `~/.arc/logs/runs.jsonl`. Both [`append`] and `doctor` use it.
pub fn path() -> Option<PathBuf> {
    Some(logs_dir()?.join("runs.jsonl"))
}

/// The file storing one run's full result, `~/.arc/logs/results/<id>.json` — read by `arc log <id>`.
pub fn result_path(id: &str) -> Option<PathBuf> {
    Some(logs_dir()?.join("results").join(format!("{id}.json")))
}

/// The record lines of a run-log `text`: non-blank lines, one JSON record each. The single
/// definition of "a record line" — both [`count`] and `arc log`'s listing build on it, so the
/// record-per-line format lives in one place rather than drifting between them.
pub fn record_lines(text: &str) -> impl Iterator<Item = &str> + '_ {
    text.lines().filter(|l| !l.trim().is_empty())
}

/// Number of run records currently logged — for `doctor`. `Ok(0)` when the log is absent (no runs
/// yet), `Ok(n)` for a readable log, and `Err` when it exists but can't be read: an unreadable log
/// is surfaced distinctly rather than silently shown as 0, which would hide a dropped/corrupt log.
pub fn count() -> std::io::Result<usize> {
    let Some(p) = path() else { return Ok(0) };
    Ok(crate::read_optional(&p)?.map_or(0, |text| record_lines(&text).count()))
}

/// Create `path`'s parent directory and run `write`, returning `Some(path)` on success. A failure
/// warns (prefixed with `what`) and returns `None` — observability writes never fail the command.
fn write_best_effort(
    path: PathBuf,
    what: &str,
    write: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Option<PathBuf> {
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write(&path)
    })();
    match result {
        Ok(()) => Some(path),
        Err(e) => {
            eprintln!("arclite: {what} ({}): {e}", path.display());
            None
        }
    }
}

/// Append `record` as one JSON line to the [`path`] run log, returning the path written.
/// Any failure is surfaced as a warning and returns `None` — logging never breaks the command.
pub fn append<T: Serialize>(record: &T) -> Option<PathBuf> {
    let Some(target) = path() else {
        eprintln!("arclite: run not logged (cannot determine the home directory)");
        return None;
    };
    let line = match serde_json::to_string(record) {
        Ok(line) => line,
        Err(e) => {
            eprintln!("arclite: run not logged (could not serialize record): {e}");
            return None;
        }
    };
    write_best_effort(target, "run not logged", |p| {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
        writeln!(file, "{line}")
    })
}

/// Store one run's full result at [`result_path`] (best-effort, like [`append`]). Returns the path
/// written, or `None` if it couldn't be stored.
pub fn store_result<T: Serialize>(id: &str, content: &T) -> Option<PathBuf> {
    let path = result_path(id)?;
    let body = match serde_json::to_string_pretty(content) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("arclite: run result not stored (could not serialize): {e}");
            return None;
        }
    };
    write_best_effort(path, "run result not stored", |p| std::fs::write(p, &body))
}
