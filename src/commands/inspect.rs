use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::cli::{GlobalArgs, InspectArgs};
use crate::output::emit;

/// Fixed manifest filenames that hint at a repo's stack/ecosystem — matched *anywhere*
/// in the tree (real repos nest them in subprojects), gitignore-aware so vendored copies
/// (e.g. `node_modules`) are excluded.
const MANIFEST_NAMES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "Gemfile",
    "composer.json",
    "CMakeLists.txt",
];

/// Manifest file *extensions* (e.g. .NET projects/solutions), reported as `*.ext`.
const MANIFEST_EXTS: &[&str] = &["csproj", "sln", "fsproj", "vbproj"];

/// How many top extensions the human `inspect` view lists before summarizing the rest.
const TOP_EXTENSIONS: usize = 10;

/// How many top-level directories the human `inspect` view lists before summarizing the rest — the
/// layout an `--include` slice aims at. A touch higher than [`TOP_EXTENSIONS`]: a repo's top-level
/// shape is the onboarding signal worth showing more of before eliding.
const TOP_DIRS: usize = 12;

/// Column width for the key (directory name or extension) in [`top_ranked`]'s block, so the counts
/// align — named like the other layout constants rather than buried in the format string.
const KEY_COLUMN_WIDTH: usize = 14;

/// Width of the top-level label column (`path`, `git repo`, `files`, …) in the human output, so the
/// values align — named (like [`KEY_COLUMN_WIDTH`] and doctor's `LABEL_WIDTH`) rather than hand-spaced
/// into the format literal.
const LABEL_WIDTH: usize = 11;

#[derive(Debug, Serialize)]
pub struct InspectReport {
    pub path: String,
    pub is_git_repo: bool,
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    pub manifests: Vec<String>,
    /// Relative paths of the manifest files actually found (root *or* nested) — what the synthesis
    /// context includes, so the scan's findings and that context can't drift apart.
    pub manifest_paths: Vec<String>,
    /// File count per top-level directory (root-level files under "."), so the view shows the layout
    /// an `--include` slice would target — not just which file *types* exist.
    pub by_top_dir: BTreeMap<String, usize>,
    pub by_extension: BTreeMap<String, usize>,
    /// Entries the walk couldn't read (permission denied, I/O, …) — counted, never silently dropped.
    pub walk_errors: usize,
}

/// Walk a repository/directory and collect structured facts, returning them with the resolved
/// absolute root (so callers reuse it rather than re-resolving).
pub fn gather(path: &Path) -> anyhow::Result<(InspectReport, PathBuf)> {
    // Distinguish absent from unreadable: exists() discards the real I/O error and reads a
    // present-but-unreadable path as merely missing; try_exists surfaces it (as the `.git` check below).
    anyhow::ensure!(
        path.try_exists()
            .with_context(|| format!("cannot access {}", path.display()))?,
        "{} does not exist",
        path.display()
    );
    let root = super::resolve_root(path)?;

    let mut files = 0usize;
    let mut dirs = 0usize;
    let mut bytes = 0u64;
    let mut by_extension: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_top_dir: BTreeMap<String, usize> = BTreeMap::new();
    let mut manifest_types: BTreeSet<String> = BTreeSet::new();
    let mut manifest_paths: Vec<String> = Vec::new();

    // Respect .gitignore, include dotfiles, but never descend into .git internals.
    // Walk errors (permission denied, I/O, …) are counted and reported, not swallowed.
    let (entries, mut walk_errors) = crate::walk::entries(&root, &root);

    for entry in entries {
        if entry.depth() == 0 {
            continue; // the root itself
        }
        let path = entry.path();
        match entry.file_type() {
            Some(ft) if ft.is_dir() => dirs += 1,
            Some(ft) if ft.is_file() => {
                files += 1;
                match entry.metadata() {
                    Ok(md) => bytes += md.len(),
                    // couldn't stat a walked file — surface it via walk_errors, don't drop its size
                    Err(_) => walk_errors += 1,
                }
                // Every walked entry lives under `root` (the root itself is skipped at depth 0), so
                // this strip cannot fail — assert the invariant rather than silently dropping a file.
                let rel = path
                    .strip_prefix(&root)
                    .expect("walked entries live under the walk root");
                let mut is_manifest = false;
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && MANIFEST_NAMES.contains(&name)
                {
                    manifest_types.insert(name.to_owned());
                    is_manifest = true;
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("(none)")
                    .to_owned();
                if MANIFEST_EXTS.contains(&ext.as_str()) {
                    manifest_types.insert(format!("*.{ext}"));
                    is_manifest = true;
                }
                if is_manifest {
                    manifest_paths.push(rel.to_string_lossy().into_owned());
                }
                // Tally the file under its top-level directory (a root-level file under ".") so the
                // view shows the layout an `--include` slice targets, not just the file types.
                let mut comps = rel.components();
                let top = match comps.next() {
                    Some(first) if comps.next().is_some() => {
                        first.as_os_str().to_string_lossy().into_owned()
                    }
                    _ => ".".to_owned(), // a file directly at the root
                };
                *by_top_dir.entry(top).or_insert(0) += 1;
                *by_extension.entry(ext).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    let manifests: Vec<String> = manifest_types.into_iter().collect();
    manifest_paths.sort();

    // `.git` present vs. unreadable are different facts: a stat error must not collapse into
    // "not a git repo" in the scan that feeds synthesis, so surface it instead of swallowing it.
    let git_dir = root.join(".git");
    let is_git_repo = git_dir
        .try_exists()
        .with_context(|| format!("cannot determine whether {} exists", git_dir.display()))?;

    let report = InspectReport {
        path: root.display().to_string(),
        is_git_repo,
        files,
        dirs,
        bytes,
        manifests,
        manifest_paths,
        by_top_dir,
        by_extension,
        walk_errors,
    };
    Ok((report, root))
}

/// Format a count-map as the inspect view's "top `n`, with the rest summarized" block — shared by the
/// top-level-directory and extension tallies so the ranking + elision logic lives in one place.
fn top_ranked(counts: &BTreeMap<String, usize>, n: usize) -> String {
    let mut ranked: Vec<(&String, &usize)> = counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut block = ranked
        .iter()
        .take(n)
        .map(|&(key, count)| format!("  {key:<width$} {count}", width = KEY_COLUMN_WIDTH))
        .collect::<Vec<_>>()
        .join("\n");
    // Surface the elision so the text view doesn't silently hide entries (--json has them all).
    match counts.len().saturating_sub(n) {
        0 => {}
        more => block.push_str(&format!("\n  … +{more} more (--json for all)")),
    }
    if block.is_empty() {
        "  (none)".to_owned()
    } else {
        block
    }
}

/// The `inspect` command.
pub fn run(args: &InspectArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (report, _root) = gather(&args.path)?;

    let row = |label: &str, value: &str| crate::labeled_row(label, value, LABEL_WIDTH);
    let mut human = [
        row("path", &report.path),
        row("git repo", &report.is_git_repo.to_string()),
        row("files", &report.files.to_string()),
        row("dirs", &report.dirs.to_string()),
        row("bytes", &report.bytes.to_string()),
        row("manifests", &crate::join_or(&report.manifests, "(none)")),
    ]
    .join("\n");
    human.push_str(&format!(
        "\ntop directories:\n{}\ntop extensions:\n{}",
        top_ranked(&report.by_top_dir, TOP_DIRS),
        top_ranked(&report.by_extension, TOP_EXTENSIONS),
    ));
    // Surface the walk's gitignore filtering so the counts above aren't read as the whole tree.
    human.push_str(&format!("\n{}", row("scope", crate::walk::SCOPE_NOTE)));
    // Surface unreadable entries (kept out of the counts above) so the scan isn't quietly partial.
    if report.walk_errors > 0 {
        human.push_str(&format!(
            "\nwalk errors {} (entries arclite couldn't read — see --json)",
            report.walk_errors
        ));
    }

    emit(&report, &human, global.json)
}
