//! `arc promote` — collect a logged run's structured findings into the run's repo `.arc/findings/`
//! ledger, one Markdown entry per finding. The **system** owns the write: agents invoke `arc promote`,
//! they never hand-author ledger entries. Each entry's name is claimed atomically with `create_new`,
//! so two sessions promoting into the same ledger at once bump a suffix instead of clobbering — the
//! concurrent-intelligence model (arc running alongside the dev agents) made safe by construction.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use crate::cli::{GlobalArgs, PromoteArgs};
use crate::output::emit;

/// One promoted finding: its ledger id and the file written (or, on a dry run, where it would go).
#[derive(Serialize)]
struct Promoted {
    id: String,
    path: String,
}

#[derive(Serialize)]
struct PromoteOutput {
    dry_run: bool,
    run: String,
    command: String,
    ledger: String,
    promoted: Vec<Promoted>,
}

/// The `promote` command.
pub fn run(args: &PromoteArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let run_id = crate::commands::log::resolve_id(&args.run)?;
    let Some(stored) = crate::commands::log::load_stored(&run_id)? else {
        anyhow::bail!(
            "no stored result for run `{run_id}` — logging was off, or it predates the result store"
        );
    };
    let record = crate::commands::log::stored_run(&stored);
    let command = record
        .get("command")
        .and_then(Value::as_str)
        .context("the stored run record has no `command`")?
        .to_owned();
    let repo = record
        .get("repo")
        .and_then(Value::as_str)
        .context("the stored run record has no `repo`, so its ledger can't be located")?;
    anyhow::ensure!(
        crate::try_is_dir(Path::new(repo))
            .with_context(|| format!("cannot access the run's repository ({repo})"))?,
        "the run's repository ({repo}) no longer exists — nothing to promote into"
    );
    // Findings are the structured `results`; a prose verb's run (summarize) has none to promote.
    let findings = stored
        .get("structured")
        .and_then(|s| s.get(crate::synth::RESULTS_KEY))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "run `{run_id}` has no structured findings to promote — its verb produces none (summarize), it predates always-structured output, or it errored before returning results"
            )
        })?;

    let ledger = crate::findings_open_dir(Path::new(repo));
    // Provenance the entries carry: the commit the run judged (when the run recorded one) and the
    // run's timestamp — so a finding's claim stays anchored to a code state and a moment, and a
    // reader in the target repo can tell whether the repo has moved on since.
    let commit = record
        .get("commit")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let recorded = record
        .get("ts")
        .and_then(Value::as_u64)
        .map(crate::commands::log::datetime_utc);

    let mut promoted = Vec::new();
    for finding in findings {
        let stem = slug(primary_text(finding));
        let path = if args.dry_run {
            // Indicative only — a real write bumps the name on a collision (see `write_entry`).
            crate::findings_entry_path(&ledger, &stem)
        } else {
            write_entry(
                &ledger,
                &stem,
                finding,
                &run_id,
                &command,
                commit.as_deref(),
                recorded.as_deref(),
            )
            .with_context(|| format!("cannot write a finding into {}", ledger.display()))?
        };
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect(
                "a promoted finding's path is built as <slug>.md, so its file stem is valid UTF-8",
            )
            .to_owned();
        promoted.push(Promoted {
            id,
            path: path.display().to_string(),
        });
    }

    // A ledger receiving its first entries also receives its explanation: an agent-facing README on
    // what the entries are, how their freshness reads, and the verify/retire lifecycle — the target
    // repo's own agents otherwise have no reason to know `.arc/findings` exists or how to treat it.
    // Best-effort: the orientation is auxiliary to the findings already written above, so a failed
    // seed warns rather than reporting the whole (successful) promotion as failed.
    let seeded_readme = if args.dry_run {
        false
    } else {
        match seed_ledger_readme(Path::new(repo)) {
            Ok(seeded) => seeded,
            Err(e) => {
                eprintln!("arclite: couldn't seed the ledger README under {repo} ({e})");
                false
            }
        }
    };

    // The runnable spelling of the verb is `arc run <verb>` (the run group, cli::NAME_RUN) — the
    // summary and the ledger entries must name a command that exists, not `arc audit`.
    let invocation = format!(
        "{} {} {command}",
        crate::cli::binary_name(),
        crate::cli::NAME_RUN
    );
    let head = format!(
        "{}{} {} finding(s) from run {run_id} (`{invocation}`) into {}:",
        if args.dry_run { "[dry run] " } else { "" },
        if args.dry_run {
            "would promote"
        } else {
            "promoted"
        },
        promoted.len(),
        crate::display_path(&ledger.display().to_string()),
    );
    let mut lines: Vec<String> = promoted
        .iter()
        .map(|p| format!("  {} · {}", p.id, crate::display_path(&p.path)))
        .collect();
    if seeded_readme {
        lines.push("  + seeded the ledger's README.md (first promotion into this repo)".to_owned());
    }
    let human = format!("{head}\n{}", lines.join("\n"));

    let out = PromoteOutput {
        dry_run: args.dry_run,
        run: run_id.clone(),
        command,
        ledger: ledger.display().to_string(),
        promoted,
    };
    emit(&out, &human, global.json)
}

/// A finding's most descriptive field — the longest string value — which names the entry. Generic
/// over the verb's item shape (audit/critique/suggest/… differ), so promote isn't coupled to any one.
fn primary_text(finding: &Value) -> &str {
    finding
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, v)| v.as_str())
        .max_by_key(|s| s.len())
        .unwrap_or("finding")
}

/// The longest a slug id may get. Caps the ledger filename so it stays recognizable and — since a
/// finding's text (e.g. an audit `location` listing many identifiers) can run to hundreds of chars —
/// clear of the platform path limit (notably Windows' ~260): an over-long name would make the entry
/// fail to *write*, not merely read badly. A curator can rename.
const SLUG_MAX_CHARS: usize = 48;

/// A kebab-case id stem from a finding's text: its leading alphanumeric words, lowercased and joined
/// with `-`, up to [`SLUG_MAX_CHARS`] on a word boundary (a single word longer than the budget is
/// truncated). Same-text findings still collide here by design — the atomic claim bumps a suffix.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for word in text
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
    {
        if out.is_empty() {
            // The first word seeds the slug, truncated if it alone exceeds the budget.
            out.extend(word.chars().take(SLUG_MAX_CHARS));
        } else if out.len() + 1 + word.len() <= SLUG_MAX_CHARS {
            out.push('-');
            out.push_str(&word);
        } else {
            break;
        }
    }
    if out.is_empty() {
        "finding".to_owned()
    } else {
        out
    }
}

/// Write one finding as a ledger entry, claimed atomically under a collision-free name via
/// [`crate::claim_findings_entry`]; the frontmatter `id` is set to the name actually claimed, so it
/// always matches the file stem.
fn write_entry(
    dir: &Path,
    stem: &str,
    finding: &Value,
    run_id: &str,
    command: &str,
    commit: Option<&str>,
    recorded: Option<&str>,
) -> std::io::Result<PathBuf> {
    let (path, mut file) = crate::claim_findings_entry(dir, stem)?;
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("a claimed entry path is <id>.md, so its file stem is valid UTF-8");
    file.write_all(entry_md(finding, id, run_id, command, commit, recorded).as_bytes())?;
    Ok(path)
}

/// One ledger entry in the seeded format: provenance frontmatter (including the commit the producing
/// run judged and when — the anchors a reader needs to tell whether the repo has moved on), the
/// finding rendered losslessly as the Claim, and the curation sections (Why/Next Action/Resolution)
/// left blank for a human or agent to fill on review — a promoted `system_run` finding is a starting
/// point, not a finished writeup. The `commit`/`recorded` lines appear only when the run carried them.
/// The frontmatter status line as promote authors it and retire rewrites it — one statement of the
/// format, so an entry-format change can't silently turn retire's replacement into a no-op.
pub(crate) const STATUS_OPEN: &str = "status: open";
pub(crate) const STATUS_RESOLVED: &str = "status: resolved";

fn entry_md(
    finding: &Value,
    id: &str,
    run_id: &str,
    command: &str,
    commit: Option<&str>,
    recorded: Option<&str>,
) -> String {
    // The same rendering the run's own output uses (synth::item_bullets), so a finding reads
    // identically in the terminal and in the ledger it was promoted into.
    let claim = crate::synth::item_bullets(finding);
    let commit_line = commit.map_or_else(String::new, |c| format!("commit: {c}\n"));
    let recorded_line = recorded.map_or_else(String::new, |r| format!("recorded: {r}\n"));
    let against = commit.map_or_else(String::new, |c| format!(" against commit `{c}`"));
    // The runnable spelling: synthesis verbs live under the run group (`arc run <verb>`), so the
    // provenance names a command a reader can actually invoke.
    let invocation = format!(
        "{} {} {command}",
        crate::cli::binary_name(),
        crate::cli::NAME_RUN
    );
    format!(
        "---\n\
         id: {id}\n\
         {STATUS_OPEN}\n\
         origin_kind: system_run\n\
         system_run_id: {run_id}\n\
         {commit_line}{recorded_line}---\n\n\
         ## Claim\n{claim}\n\n\
         ## Evidence\nPromoted from `{invocation}` run `{run_id}`{against} — see `arc log {run_id}` for the full run and its note.\n\n\
         ## Why It Matters\n\n\
         ## Next Action\n\n\
         ## Resolution\n"
    )
}

/// The agent-facing explanation seeded into a ledger's first promotion — what the entries are, how
/// their freshness reads, and the lifecycle — kept general (it lands in arbitrary repos), and
/// self-dated with the arc that wrote it: a generated orientation must say when it was true and
/// yield to the installed `arc`, never read as canon after arc evolves past it.
fn ledger_readme() -> String {
    format!(
        "\
# Findings Ledger

Findings this repository carries forward, written by `arc promote` from logged runs (possibly run
from outside this repo) and moved to `resolved/` by `arc retire`. The system owns the entries:
react to one, fix the code, or retire it — don't hand-edit the files.

Each entry's frontmatter:

- `status` — `open` until retired.
- `system_run_id` — the producing run; `arc log <id>` re-shows its full context and note.
- `commit` — the commit the run judged; a `-dirty` suffix means the worktree had changes beyond it.
  The claim is about that state — if the repo has moved on, re-check before acting.
- `recorded` — when the run happened.

Re-checking: `arc run verify <repo>` judges every open finding against the current code
(`reproduces` / `resolved` / `indeterminate`), and `arc retire <verify-run-id>` moves the resolved
ones out. A newer entry about the same issue supersedes an older one — trust recency.

---

Seeded by `arc` v{} on {}, at this ledger's first promotion. A generated orientation, not canon:
arc and this format may have evolved since — where they disagree, the installed `arc` (`arc --help`)
is authoritative. Edit or delete freely; arc never rewrites this file.
",
        env!("CARGO_PKG_VERSION"),
        crate::commands::log::datetime_utc(crate::log::now_secs()),
    )
}

/// Seed [`ledger_readme`] into `<repo>/.arc/findings/` when absent, returning whether it was written.
/// Never overwrites — a repo's own curated README wins; present-but-unreadable propagates rather than
/// masquerading as already-seeded.
fn seed_ledger_readme(repo: &Path) -> std::io::Result<bool> {
    let path = crate::findings_open_dir(repo)
        .parent()
        .expect("the open ledger always sits inside .arc/findings")
        .join("README.md");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(ledger_readme().as_bytes())?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{SLUG_MAX_CHARS, slug};

    #[test]
    fn slug_stays_within_the_path_budget_on_a_word_boundary() {
        // A finding whose text is many long identifier-words (an audit `location`) must not yield a
        // filename that risks the platform path limit; the slug caps in length, breaking between words.
        let s = slug(
            "PairThesisController citationcontext handlers GetTranscriptCitationContextAsync GetFilingCitationContextAsync",
        );
        assert!(s.len() <= SLUG_MAX_CHARS, "`{s}` exceeds {SLUG_MAX_CHARS}");
        assert!(
            !s.starts_with('-') && !s.ends_with('-'),
            "`{s}` has a stray boundary dash"
        );
    }

    #[test]
    fn slug_truncates_a_single_oversized_word() {
        assert_eq!(slug(&"x".repeat(200)).len(), SLUG_MAX_CHARS);
    }

    #[test]
    fn slug_falls_back_when_text_has_no_alphanumerics() {
        assert_eq!(slug("—— ·· ——"), "finding");
    }
}
