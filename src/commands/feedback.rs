//! `arc feedback` — the feedback channel: one verb, two sinks. An **outbound** report (the
//! default) is captured locally under `~/.arc/feedback.jsonl` — collection first, transport later
//! (a euhedron endpoint is the rulespace registry's future; `--issue` prints a prefilled GitHub
//! issue URL as the zero-infrastructure interim). An **inbox** note (`--inbox`) is queued into the
//! target repo's `.arc/inbox/` — one Markdown file per note, the items-ledger pattern — for the
//! next session to read and triage: the agenda's lighter-weight cousin. The TUI's feedback view
//! captures through these same functions, so there is one write path per sink.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::cli::{FeedbackArgs, GlobalArgs};
use crate::output::emit;

/// Seeded into a repo's `.arc/inbox/` on first use (atomically, never overwriting), so the
/// directory explains its own triage contract to whichever session finds it.
const INBOX_README: &str = "# Inbox\n\nRaw notes queued for future sessions — `arc feedback --inbox \"<note>\"` writes one file per\nnote. A session triages each: act on it, convert it (an agenda item, a finding, a memory), or\ndrop it — then deletes the file. Deliberately pre-agenda and unstructured; the items ledger is\nwhere triage lands.\n";

/// One captured outbound report, as recorded in `~/.arc/feedback.jsonl`.
#[derive(serde::Serialize)]
struct Report<'a> {
    id: &'a str,
    ts: u64,
    version: &'static str,
    build: &'static str,
    os: &'static str,
    arch: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<&'a str>,
}

pub fn run(args: &FeedbackArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let message = args.message.trim();
    anyhow::ensure!(!message.is_empty(), "the feedback message is empty");
    // A run reference must be a usable id before it's recorded — a mangled one would dangle
    // forever in the report it's meant to ground.
    if let Some(run) = args.run.as_deref() {
        crate::commands::log::ensure_safe_run_id(run)?;
    }
    let id = crate::log::new_id();
    if args.inbox {
        let path = inbox_note(&args.path, &id, message, args.run.as_deref())?;
        let human = format!("queued for the next session: {}", path.display());
        let payload = serde_json::json!({ "id": id, "inbox": path.display().to_string() });
        return emit(&payload, &human, global.json);
    }
    let path = capture(&id, message, args.run.as_deref())?;
    let mut human = format!("captured: {id} -> {}", path.display());
    let issue = args.issue.then(|| issue_url(message));
    if let Some(url) = &issue {
        human.push_str(&format!("\nfile it upstream (prefilled): {url}"));
    }
    let payload = serde_json::json!({
        "id": id,
        "captured": path.display().to_string(),
        "issue_url": issue,
    });
    emit(&payload, &human, global.json)
}

/// The outbound queue, `~/.arc/feedback.jsonl` — its own domain beside `logs/`, not inside it (a
/// report is authored, not a run trace).
pub(crate) fn queue_path() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("feedback.jsonl"))
}

/// Append one outbound report to the queue. Unlike run logging (best-effort — logging must never
/// fail the run), capture *is* the command: a report that can't be written is an error, never a
/// warning-and-shrug.
pub(crate) fn capture(id: &str, message: &str, run: Option<&str>) -> anyhow::Result<PathBuf> {
    let path =
        queue_path().context("cannot determine the home directory for the feedback queue")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {} for the feedback queue", dir.display()))?;
    }
    let record = Report {
        id,
        ts: crate::log::now_secs(),
        version: env!("CARGO_PKG_VERSION"),
        build: env!("ARC_BUILD_COMMIT"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        message,
        run,
    };
    let line = format!(
        "{}\n",
        serde_json::to_string(&record).expect("a report serializes")
    );
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening the feedback queue at {}", path.display()))?;
    // One write of line + newline, the run log's O_APPEND no-interleave idiom (see log::append).
    let n = file.write(line.as_bytes())?;
    anyhow::ensure!(
        n == line.len(),
        "partial append ({n} of {} bytes) to {}",
        line.len(),
        path.display()
    );
    Ok(path)
}

/// Queue one inbox note into `<repo>/.arc/inbox/<id>.md` — the whole file is the note (the items
/// pattern), claimed with an exclusive create so concurrent captures can't collide. Seeds the
/// directory's README on first use.
pub(crate) fn inbox_note(
    repo: &Path,
    id: &str,
    message: &str,
    run: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let dir = repo.join(crate::ARC_DIR).join("inbox");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("README.md"))
    {
        Ok(mut f) => f
            .write_all(INBOX_README.as_bytes())
            .context("seeding the inbox README")?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(anyhow::Error::new(e).context("seeding the inbox README")),
    }
    let path = dir.join(format!("{id}.md"));
    let body = match run {
        Some(run) => format!("{message}\n\n(run: {run})\n"),
        None => format!("{message}\n"),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("claiming {}", path.display()))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// The prefilled GitHub issue URL for filing a report upstream — the interim outbound transport.
/// Carries the message plus the running arc's identity, and deliberately nothing else: a run's
/// mechanics (paths, spend, other repos) are the operator's, and an issue is public.
pub(crate) fn issue_url(message: &str) -> String {
    let title: String = message.chars().take(72).collect();
    let body = format!(
        "{message}\n\n---\narc {} ({}) - {}-{}",
        env!("CARGO_PKG_VERSION"),
        env!("ARC_BUILD_COMMIT"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    format!(
        "{}/{}/{}/issues/new?title={}&body={}",
        crate::commands::update::HOST,
        crate::commands::update::OWNER,
        crate::commands::update::REPO,
        percent_encode(&title),
        percent_encode(&body),
    )
}

/// Percent-encode for a URL query value — everything but RFC 3986 unreserved, so the message can't
/// escape its parameter slot regardless of content.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The outbound queue's records (newest first) plus the count of unparsable lines — disclosed,
/// never silently dropped (the run log's reader contract). An absent queue is an empty one.
pub(crate) fn reports_newest_first() -> anyhow::Result<(Vec<serde_json::Value>, usize)> {
    let Some(path) = queue_path() else {
        anyhow::bail!("cannot determine the home directory for the feedback queue");
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("reading the feedback queue at {}", path.display())));
        }
    };
    let mut reports = Vec::new();
    let mut unparsed = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => reports.push(v),
            Err(_) => unparsed += 1,
        }
    }
    reports.reverse();
    Ok((reports, unparsed))
}

/// A repo's inbox notes as (file name, first line), newest first by id-shaped name — the listing
/// the TUI's feedback view shows beside the outbound queue. An absent inbox is an empty one.
pub(crate) fn inbox_notes(repo: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let dir = repo.join(crate::ARC_DIR).join("inbox");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("reading {}", dir.display())));
        }
    };
    let mut notes = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".md") else {
            continue;
        };
        if stem == "README" {
            continue;
        }
        let first = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        notes.push((stem.to_owned(), first));
    }
    // Newest first by the id's parsed `<secs>-<pid>-<nanos>` segments — compared numerically, so a
    // shorter segment can't out-sort a longer one the way raw string order would. Non-conforming
    // names order after the conforming ones, by name; the order is imposed deterministically so
    // directory-iteration order can't leak through.
    notes.sort_by(|a, b| match (id_sort_key(&a.0), id_sort_key(&b.0)) {
        (Some(ka), Some(kb)) => kb.cmp(&ka),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    Ok(notes)
}

/// An id's `<secs>-<pid>-<nanos>` segments parsed for ordering; `None` for a name not of that
/// shape (see the sort above for how those order).
fn id_sort_key(id: &str) -> Option<(u64, u64, u64)> {
    let mut parts = id.splitn(3, '-');
    let key = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    Some(key)
}
