//! `arc retire <verify-run-id>` — act on a verify run's verdicts: move each `resolved` finding out of
//! the open ledger into `.arc/findings/resolved/`, marked resolved with the verdict's provenance. The
//! **system** owns the move (agents invoke `arc retire`; they don't hand-edit the ledger), mirroring
//! `promote`'s system-writes-the-entry discipline — the closing end of the findings lifecycle.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use crate::cli::{GlobalArgs, RetireArgs};
use crate::output::emit;

/// One retired finding: its id and the move (open → resolved); on a dry run, where it would move to.
#[derive(Serialize)]
struct Retired {
    id: String,
    from: String,
    to: String,
}

#[derive(Serialize)]
struct RetireOutput {
    dry_run: bool,
    run: String,
    repo: String,
    retired: Vec<Retired>,
    /// `resolved` verdicts whose id matched no open ledger entry (already retired, or a drifted id) —
    /// surfaced rather than silently dropped.
    unmatched: Vec<String>,
}

/// The `retire` command.
pub fn run(args: &RetireArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let run_id = crate::commands::log::resolve_id(&args.run)?;
    let Some(stored) = crate::commands::log::load_stored(&run_id)? else {
        anyhow::bail!(
            "no stored result for run `{run_id}` — logging was off, or it predates the result store"
        );
    };
    let record = crate::commands::log::stored_run(&stored);
    let repo = record
        .get("repo")
        .and_then(Value::as_str)
        .context("the stored run record has no `repo`, so its ledger can't be located")?;
    anyhow::ensure!(
        crate::try_is_dir(Path::new(repo))
            .with_context(|| format!("cannot access the run's repository ({repo})"))?,
        "the run's repository ({repo}) no longer exists — nothing to retire from"
    );
    // The verdicts are the structured `results` (a verify run emits {id, verdict, reason}); a prose run
    // has none. We act only on `resolved` — `reproduces`/`indeterminate` stay in the open ledger.
    let verdicts = stored
        .get("structured")
        .and_then(|s| s.get(crate::synth::RESULTS_KEY))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "run `{run_id}` has no structured verdicts — retire acts on an `arc run verify --structured` run"
            )
        })?;

    let open = crate::findings_open_dir(Path::new(repo));
    let resolved = crate::findings_resolved_dir(Path::new(repo));

    let mut retired = Vec::new();
    let mut unmatched = Vec::new();
    for v in verdicts {
        if v.get("verdict").and_then(Value::as_str) != Some("resolved") {
            continue; // only resolved findings retire; reproduces/indeterminate stay open
        }
        let Some(id) = v
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue; // a verdict with no id names no ledger entry
        };
        // The id rode through the model — validate it as an untrusted path segment before joining it to
        // the ledger dir, reusing the canonical single-safe-segment check (shared with run-id validation).
        if crate::commands::log::ensure_safe_run_id(id).is_err() {
            unmatched.push(id.to_owned());
            continue;
        }
        let src = open.join(format!("{id}.md"));
        // Absent → no matching open entry (already retired, or a drifted id); unreadable → a real error.
        let present = crate::optional(std::fs::metadata(&src))
            .with_context(|| format!("checking open finding {}", src.display()))?
            .is_some_and(|m| m.is_file());
        if !present {
            unmatched.push(id.to_owned());
            continue;
        }
        let reason = v.get("reason").and_then(Value::as_str).unwrap_or("");
        let dest = if args.dry_run {
            // indicative; a real move bumps on collision (see `move_entry`)
            crate::findings_entry_path(&resolved, id)
        } else {
            move_entry(&src, &resolved, id, reason, &run_id).with_context(|| {
                format!("cannot retire finding `{id}` into {}", resolved.display())
            })?
        };
        retired.push(Retired {
            id: id.to_owned(),
            from: src.display().to_string(),
            to: dest.display().to_string(),
        });
    }

    let head = format!(
        "{}{} {} resolved finding(s) from verify run {run_id} → {}{}",
        if args.dry_run { "[dry run] " } else { "" },
        if args.dry_run {
            "would retire"
        } else {
            "retired"
        },
        retired.len(),
        crate::display_path(&resolved.display().to_string()),
        if unmatched.is_empty() {
            String::new()
        } else {
            format!(
                " ({} resolved verdict(s) matched no open entry: {})",
                unmatched.len(),
                unmatched.join(", ")
            )
        },
    );
    let lines: Vec<String> = retired
        .iter()
        .map(|r| {
            format!(
                "  {} · {} → {}",
                r.id,
                crate::display_path(&r.from),
                crate::display_path(&r.to)
            )
        })
        .collect();
    let human = if lines.is_empty() {
        head
    } else {
        format!("{head}\n{}", lines.join("\n"))
    };

    let out = RetireOutput {
        dry_run: args.dry_run,
        run: run_id,
        repo: repo.to_owned(),
        retired,
        unmatched,
    };
    emit(&out, &human, global.json)
}

/// Move one finding from the open ledger into the resolved dir, marked resolved with the verify run's
/// provenance. Write-new-then-remove-old: the resolved copy is claimed via [`crate::claim_findings_entry`]
/// and fully written before the open entry is removed, so a failure mid-move never loses the finding — at
/// worst it lingers in both dirs, which a re-run reconciles.
fn move_entry(
    src: &Path,
    dir: &Path,
    id: &str,
    reason: &str,
    run_id: &str,
) -> std::io::Result<PathBuf> {
    let resolved_body = mark_resolved(&std::fs::read_to_string(src)?, reason, run_id);
    let (dest, mut file) = crate::claim_findings_entry(dir, id)?;
    file.write_all(resolved_body.as_bytes())?;
    // Reached only after the resolved copy is safely written, so the finding is never lost; a failed
    // remove leaves a reconcilable duplicate rather than a gap.
    std::fs::remove_file(src)?;
    Ok(dest)
}

/// Flip a finding's frontmatter `status:` to `resolved` and append the verify run's verdict to its
/// Resolution section, so the moved entry is self-describing and carries durable provenance (the run
/// id, not an agent-recorded timestamp — see `.arc/findings/README.md`).
fn mark_resolved(body: &str, reason: &str, run_id: &str) -> String {
    let restatused = body.replacen("status: open", "status: resolved", 1);
    let note = if reason.is_empty() {
        format!("Resolved per verify run `{run_id}`.")
    } else {
        format!("Resolved per verify run `{run_id}`: {reason}")
    };
    // Land the note under the existing Resolution heading (the seeded format ends with it) or add one —
    // either way without assuming the file's exact structure.
    if restatused.contains("## Resolution") {
        format!("{}\n{note}\n", restatused.trim_end())
    } else {
        format!("{}\n\n## Resolution\n{note}\n", restatused.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_resolved_flips_status_and_appends_under_existing_resolution() {
        let body = "---\nid: x\nstatus: open\n---\n\n## Claim\nthing\n\n## Resolution\n";
        let out = mark_resolved(body, "no longer present", "run-1");
        assert!(out.contains("status: resolved"));
        assert!(!out.contains("status: open"));
        assert!(out.contains("Resolved per verify run `run-1`: no longer present"));
        assert_eq!(out.matches("## Resolution").count(), 1);
    }

    #[test]
    fn mark_resolved_adds_a_resolution_section_when_absent() {
        let body = "---\nstatus: open\n---\n\n## Claim\nthing\n";
        let out = mark_resolved(body, "", "run-2");
        assert_eq!(out.matches("## Resolution").count(), 1);
        assert!(out.contains("Resolved per verify run `run-2`."));
    }
}
