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
    // Distinguish absent from unreadable: an unreadable repo path must surface, not read as "gone".
    anyhow::ensure!(
        crate::try_is_dir(Path::new(repo))
            .with_context(|| format!("cannot access the run's repository ({repo})"))?,
        "the run's repository ({repo}) no longer exists — nothing to promote into"
    );
    // Findings are the structured `results`; a prose run (no `--structured`) has none to promote.
    let findings = stored
        .get("structured")
        .and_then(|s| s.get(crate::synth::RESULTS_KEY))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "run `{run_id}` has no structured findings to promote — re-run the verb with `--structured` (or `--fail-on-findings`)"
            )
        })?;

    let ledger = Path::new(repo)
        .join(crate::ARC_DIR)
        .join("findings")
        .join("open");

    let mut promoted = Vec::new();
    for finding in findings {
        let stem = slug(primary_text(finding));
        let path = if args.dry_run {
            // Indicative only — a real write bumps the name on a collision (see `write_entry`).
            ledger.join(format!("{stem}.md"))
        } else {
            write_entry(&ledger, &stem, finding, &run_id, &command)
                .with_context(|| format!("cannot write a finding into {}", ledger.display()))?
        };
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&stem)
            .to_owned();
        promoted.push(Promoted {
            id,
            path: path.display().to_string(),
        });
    }

    let head = format!(
        "{}{} {} finding(s) from run {run_id} (`arc {command}`) into {}:",
        if args.dry_run { "[dry run] " } else { "" },
        if args.dry_run {
            "would promote"
        } else {
            "promoted"
        },
        promoted.len(),
        crate::display_path(&ledger.display().to_string()),
    );
    let lines: Vec<String> = promoted
        .iter()
        .map(|p| format!("  {} · {}", p.id, crate::display_path(&p.path)))
        .collect();
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

/// How many leading words of a finding's text form its slug id — enough to be recognizable without an
/// unwieldy filename (a curator can rename).
const SLUG_WORDS: usize = 8;

/// A kebab-case id stem from a finding's text: its first [`SLUG_WORDS`] alphanumeric words, lowercased.
fn slug(text: &str) -> String {
    let s = text
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .take(SLUG_WORDS)
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "finding".to_owned()
    } else {
        s
    }
}

/// Write one finding as a ledger entry under a collision-free name, claimed atomically: `create_new`
/// fails if the name exists, so concurrent promotes bump a numeric suffix rather than clobber. The
/// frontmatter `id` is set to the name actually claimed, so it always matches the file stem.
fn write_entry(
    dir: &Path,
    stem: &str,
    finding: &Value,
    run_id: &str,
    command: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut n = 0u32;
    loop {
        let id = if n == 0 {
            stem.to_owned()
        } else {
            format!("{stem}-{n}")
        };
        let path = dir.join(format!("{id}.md"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(entry_md(finding, &id, run_id, command).as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// One ledger entry in the seeded format: provenance frontmatter, the finding rendered losslessly as
/// the Claim, and the curation sections (Why/Next Action/Resolution) left blank for a human or agent
/// to fill on review — a promoted `system_run` finding is a starting point, not a finished writeup.
fn entry_md(finding: &Value, id: &str, run_id: &str, command: &str) -> String {
    let claim = finding
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            let val = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            format!("- **{k}:** {val}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "---\n\
         id: {id}\n\
         status: open\n\
         origin_kind: system_run\n\
         system_run_id: {run_id}\n\
         ---\n\n\
         ## Claim\n{claim}\n\n\
         ## Evidence\nPromoted from `arc {command}` run `{run_id}` — see `arc log {run_id}` for the full run and its note.\n\n\
         ## Why It Matters\n\n\
         ## Next Action\n\n\
         ## Resolution\n"
    )
}
