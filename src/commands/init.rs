//! `arc init`: scaffold the `.arc` config (and, with `--hook`, an opt-in pre-push gate) for a repo.

use std::path::Path;

use anyhow::Context;
use serde::Serialize;

use crate::cli::{GlobalArgs, InitArgs};
use crate::output::emit;

/// Starter project settings: a `project` ruleset sourcing `.arc/rules`, set as the default, so the
/// AI commands weigh the repo's own rules without further setup.
const STARTER_SETTINGS: &str = r#"{
  "defaults": { "ruleset": "project" },
  "rulesets": { "project": { "sources": ["rules"] } }
}
"#;

/// Starter pre-push gate — a minimal composition the repo edits to taste.
const STARTER_HOOK: &str = r#"#!/bin/sh
# arclite gate (pre-push). Edit the command(s) below to taste; skip once with `ARC_GATE=0 git push`.
if [ "$ARC_GATE" = "0" ]; then exit 0; fi
arc audit --fail-on-findings
"#;

#[derive(Serialize)]
struct InitReport {
    created: Vec<String>,
    skipped: Vec<String>,
}

/// Scaffold arclite's per-repo files, never clobbering what's already there.
pub fn run(args: &InitArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let root = std::path::absolute(&args.path)
        .with_context(|| format!("cannot resolve {}", args.path.display()))?;
    let arc = root.join(crate::ARC_DIR);
    let mut created = Vec::new();
    let mut skipped = Vec::new();

    ensure_dir(&arc.join("rules"), &mut created, &mut skipped)?;
    write_if_absent(
        &arc.join("settings.json"),
        STARTER_SETTINGS,
        &mut created,
        &mut skipped,
    )?;

    if args.hook {
        let hooks = root.join("hooks");
        std::fs::create_dir_all(&hooks)
            .with_context(|| format!("cannot create {}", hooks.display()))?;
        let hook = hooks.join("pre-push");
        if write_if_absent(&hook, STARTER_HOOK, &mut created, &mut skipped)? {
            make_executable(&hook)?;
        }
        activate_hooks(&root)?;
    }

    let human = format!(
        "created: {}\nskipped: {}",
        join_or_none(&created),
        join_or_none(&skipped)
    );
    emit(&InitReport { created, skipped }, &human, global.json)
}

fn join_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_owned()
    } else {
        items.join(", ")
    }
}

/// Create `dir` if absent, recording which happened.
fn ensure_dir(
    dir: &Path,
    created: &mut Vec<String>,
    skipped: &mut Vec<String>,
) -> anyhow::Result<()> {
    if dir.exists() {
        skipped.push(dir.display().to_string());
    } else {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        created.push(dir.display().to_string());
    }
    Ok(())
}

/// Write `content` to `path` only if absent (never clobber), returning whether it was written.
/// The parent directory must already exist.
fn write_if_absent(
    path: &Path,
    content: &str,
    created: &mut Vec<String>,
    skipped: &mut Vec<String>,
) -> anyhow::Result<bool> {
    if path.exists() {
        skipped.push(path.display().to_string());
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("cannot write {}", path.display()))?;
    created.push(path.display().to_string());
    Ok(true)
}

/// Point git at the committed `hooks/` directory so the pre-push gate runs — the opt-in activation.
fn activate_hooks(root: &Path) -> anyhow::Result<()> {
    let ok = crate::ai::command("git")
        .current_dir(root)
        .args(["config", "core.hooksPath", "hooks"])
        .status()
        .context("could not run git to set core.hooksPath")?
        .success();
    anyhow::ensure!(
        ok,
        "git config core.hooksPath failed (is {} a git repository?)",
        root.display()
    );
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755); // rwxr-xr-x — a git hook must be executable to run
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("cannot set the executable bit on {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(()) // on Windows, git runs the hook through sh regardless of the executable bit
}
