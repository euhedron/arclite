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
    // Retire acts on verify verdicts specifically. Another verb's structured results can carry the
    // same field names by coincidence (or a model's drift), and acting on those would move ledger
    // entries on a judgment that never re-checked them — so the run's identity is checked, not shaped.
    let command = record.get("command").and_then(Value::as_str).unwrap_or("");
    anyhow::ensure!(
        command == crate::cli::NAME_VERIFY,
        "run `{run_id}` is a `{command}` run — retire acts on `arc run verify` verdicts"
    );
    let repo = record
        .get("repo")
        .and_then(Value::as_str)
        .context("the stored run record has no `repo`, so its ledger can't be located")?;
    anyhow::ensure!(
        crate::try_is_dir(Path::new(repo))
            .with_context(|| format!("cannot access the run's repository ({repo})"))?,
        "the run's repository ({repo}) no longer exists — nothing to retire from"
    );
    // The verdicts are the structured `results` (a verify run emits {id, verdict, reason}). Absent
    // structure and legitimately-empty results are distinct failures, told apart honestly: verify is
    // structured by default, so no `results` at all means the store predates structure or the run
    // errored — while an empty array means the verify ran with no open findings in context.
    let verdicts = stored
        .get("structured")
        .and_then(|s| s.get(crate::synth::RESULTS_KEY))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "run `{run_id}` carries no structured verdicts — it predates always-structured output, or errored before returning results"
            )
        })?;
    anyhow::ensure!(
        !verdicts.is_empty(),
        "verify run `{run_id}` recorded no verdicts — its context carried no open findings; nothing to retire"
    );

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
            // The same collision-aware sequence the real claim walks, probed without writing — the
            // preview names the path a run started now would take (indicative under concurrency).
            crate::preview_findings_entry(&resolved, id)
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
    if let Err(e) = file.write_all(resolved_body.as_bytes()) {
        // A half-written resolved copy must not linger: a later retry would suffix past it, leaving
        // a partial duplicate in the ledger. Roll the claim back (best-effort — a failed cleanup is
        // named in the error, and the open entry is still intact either way).
        drop(file);
        if let Err(rm) = std::fs::remove_file(&dest) {
            return Err(std::io::Error::new(
                e.kind(),
                format!(
                    "{e} (and the partial {} could not be removed: {rm})",
                    dest.display()
                ),
            ));
        }
        return Err(e);
    }
    // Reached only after the resolved copy is safely written, so the finding is never lost. A failed
    // source removal rolls the whole move back (removes the resolved copy): the ledger is left
    // either fully moved or not moved at all — never holding both copies for a re-run to suffix
    // past instead of reconcile. A rollback that itself fails is named in the error.
    if let Err(e) = std::fs::remove_file(src) {
        if let Err(rm) = std::fs::remove_file(&dest) {
            return Err(std::io::Error::new(
                e.kind(),
                format!(
                    "{e} (and rolling back the resolved copy {} failed: {rm} — both copies remain)",
                    dest.display()
                ),
            ));
        }
        return Err(e);
    }
    Ok(dest)
}

/// Flip a finding's frontmatter `status:` to `resolved` and append the verify run's verdict to its
/// Resolution section, so the moved entry is self-describing and carries durable provenance (the run
/// id, not an agent-recorded timestamp — see `.arc/findings/README.md`).
fn mark_resolved(body: &str, reason: &str, run_id: &str) -> String {
    // Rewrite the `status:` field by structure — keyed on the field name, inside the `---`-delimited
    // frontmatter block only — not a substring search over the whole document. Any current value
    // flips (open, accepted, a hand-authored variant), a body that *quotes* a status line is left
    // alone, and an entry with no status field is warned about rather than silently no-opped into
    // `resolved/` still reading `open`. The replacement line is promote's constant — the one
    // statement of the entry format.
    let mut in_frontmatter = false;
    let mut replaced = false;
    let mut lines: Vec<&str> = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if line.trim_end() == "---" {
            // Entering on the leading delimiter, leaving on the closing one.
            in_frontmatter = i == 0;
            lines.push(line);
            continue;
        }
        if in_frontmatter && !replaced && line.trim_start().starts_with("status:") {
            lines.push(super::promote::STATUS_RESOLVED);
            replaced = true;
            continue;
        }
        lines.push(line);
    }
    if !replaced {
        eprintln!(
            "arclite: retired entry carries no frontmatter `status:` field to flip — moved as-is"
        );
    }
    let note = if reason.is_empty() {
        format!("Resolved per verify run `{run_id}`.")
    } else {
        format!("Resolved per verify run `{run_id}`: {reason}")
    };
    // Land the note inside the Resolution *section* — at its end, before the next `## ` heading —
    // not at the file's end, which would file the note under whatever section happens to be last.
    // No heading at all → add the section at the end.
    let mut lines: Vec<String> = lines.into_iter().map(str::to_owned).collect();
    if let Some(h) = lines.iter().position(|l| l.trim_end() == "## Resolution") {
        let mut at = lines[h + 1..]
            .iter()
            .position(|l| l.starts_with("## "))
            .map_or(lines.len(), |off| h + 1 + off);
        // Step back over the section's trailing blank lines so the note sits flush under its content.
        while at > h + 1 && lines[at - 1].trim().is_empty() {
            at -= 1;
        }
        lines.insert(at, note);
    } else {
        lines.push(String::new());
        lines.push("## Resolution".to_owned());
        lines.push(note);
    }
    format!("{}\n", lines.join("\n").trim_end())
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

    #[test]
    fn mark_resolved_flips_any_status_value_not_just_open() {
        let body = "---\nid: x\nstatus: accepted\n---\n\n## Claim\nthing\n";
        let out = mark_resolved(body, "", "run-3");
        assert!(out.contains("status: resolved"));
        assert!(!out.contains("status: accepted"));
    }

    #[test]
    fn mark_resolved_leaves_a_status_line_quoted_in_the_body_alone() {
        let body =
            "---\nid: x\nstatus: open\n---\n\n## Claim\nfrontmatter says `status: open` today\n";
        let out = mark_resolved(body, "", "run-4");
        assert!(out.contains("status: resolved"));
        // The body's quoted mention is untouched — only the frontmatter field flipped.
        assert!(out.contains("frontmatter says `status: open` today"));
    }

    #[test]
    fn mark_resolved_lands_the_note_inside_the_resolution_section_not_at_file_end() {
        let body =
            "---\nstatus: open\n---\n\n## Claim\nthing\n\n## Resolution\n\n## Next Action\nlater\n";
        let out = mark_resolved(body, "", "run-5");
        let note_at = out.find("Resolved per verify run `run-5`.").unwrap();
        let next_at = out.find("## Next Action").unwrap();
        // The note belongs to Resolution — before the following section, not appended after it.
        assert!(note_at < next_at);
    }
}
