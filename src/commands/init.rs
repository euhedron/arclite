use std::path::Path;

use anyhow::Context;
use serde::Serialize;

use crate::cli::{GlobalArgs, InitArgs};
use crate::output::emit;
use crate::settings::Settings;

/// The rules subdirectory inside `.arc` — one name for the directory the scaffold creates and the
/// starter ruleset's source, which must agree or the scaffolded default silently resolves to nothing.
const RULES_DIR: &str = "rules";

/// The hooks subdirectory inside `.arc` — the directory the scaffold writes; the `core.hooksPath`
/// value that activates it is derived as `<ARC_DIR>/<this>` in [`activate_hooks`], so the scaffolded
/// directory and the activation can't drift.
const HOOKS_SUBDIR: &str = "hooks";

/// The scaffolded ruleset's name — referenced twice in the starter settings (as the configured
/// default and as the ruleset's key), which must agree or the default resolves to nothing.
const PROJECT_RULESET: &str = "project";

/// Starter project settings: a [`PROJECT_RULESET`] ruleset sourcing `.arc/rules` ([`RULES_DIR`]),
/// set as the default, so the AI commands weigh the repo's own rules without further setup.
fn starter_settings() -> String {
    format!(
        r#"{{
  "defaults": {{ "ruleset": "{PROJECT_RULESET}" }},
  "rulesets": {{ "{PROJECT_RULESET}": {{ "sources": ["{RULES_DIR}"] }} }}
}}
"#
    )
}

/// Starter pre-push gate — a minimal composition the repo edits to taste. The binary name comes from
/// [`crate::cli::binary_name`] (the single source `doctor` detects the hook by) rather than a literal,
/// so a rename can't desync the scaffolded hook from arc's own detection.
fn starter_hook() -> String {
    format!(
        "#!/bin/sh\n\
         # arclite gate (pre-push). Edit the command(s) below to taste; skip once with `ARC_GATE=0 git push`.\n\
         if [ \"$ARC_GATE\" = \"0\" ]; then exit 0; fi\n\
         {} run audit --fail-on-findings\n",
        crate::cli::binary_name()
    )
}

#[derive(Serialize)]
struct InitReport {
    created: Vec<String>,
    skipped: Vec<String>,
}

/// The `init` command — never clobbers what's already there.
pub fn run(args: &InitArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let root = super::resolve_root(&args.path)?;
    anyhow::ensure!(
        crate::try_is_dir(&root).with_context(|| format!("cannot access {}", root.display()))?,
        "cannot initialize {}: not an existing directory (init sets up `.arc` in a repo that already exists, so a typo'd path doesn't scaffold a stray tree)",
        root.display()
    );
    let arc = root.join(crate::ARC_DIR);
    let mut created = Vec::new();
    let mut skipped = Vec::new();

    ensure_dir(&arc.join(RULES_DIR), &mut created, &mut skipped)?;
    write_if_absent(
        &Settings::project_path(&root),
        &starter_settings(),
        &mut created,
        &mut skipped,
    )?;

    if args.hook {
        let hooks = arc.join(HOOKS_SUBDIR);
        std::fs::create_dir_all(&hooks)
            .with_context(|| format!("cannot create {}", hooks.display()))?;
        let hook = hooks.join("pre-push");
        if write_if_absent(&hook, &starter_hook(), &mut created, &mut skipped)? {
            make_executable(&hook)?;
        }
        activate_hooks(&root)?;
    }

    let human = format!(
        "created: {}\nskipped: {}",
        crate::join_or(&created, "(none)"),
        crate::join_or(&skipped, "(none)")
    );
    emit(&InitReport { created, skipped }, &human, global.json)
}

/// Create `dir` if absent, recording which happened.
fn ensure_dir(
    dir: &Path,
    created: &mut Vec<String>,
    skipped: &mut Vec<String>,
) -> anyhow::Result<()> {
    if dir
        .try_exists()
        .with_context(|| format!("cannot access {}", dir.display()))?
    {
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
    if path
        .try_exists()
        .with_context(|| format!("cannot access {}", path.display()))?
    {
        skipped.push(path.display().to_string());
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("cannot write {}", path.display()))?;
    created.push(path.display().to_string());
    Ok(true)
}

/// Point git at the committed `.arc/hooks` directory so the pre-push gate runs — the opt-in activation.
fn activate_hooks(root: &Path) -> anyhow::Result<()> {
    // git wants core.hooksPath as a forward-slash relative path; derive it from ARC_DIR + the subdir so
    // it tracks the directory the scaffold actually wrote.
    let hooks_path = format!("{}/{HOOKS_SUBDIR}", crate::ARC_DIR);
    // Don't clobber a core.hooksPath the user already set for something else — surface it and stop,
    // rather than silently overwriting (the scaffold is careful never to clobber files; so is this).
    let current = crate::git_config_get(root, "core.hooksPath")?.unwrap_or_default();
    anyhow::ensure!(
        current.is_empty() || current == hooks_path,
        "core.hooksPath is already set to `{current}` — leaving it untouched; set it to `{hooks_path}` \
         (or unset it) yourself to activate the arclite gate"
    );
    let ok = crate::ai::command("git")?
        .current_dir(root)
        .args(["config", "core.hooksPath", hooks_path.as_str()])
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
    perms.set_mode(0o755); // a git hook must be executable to run
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("cannot set the executable bit on {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(()) // on Windows, git runs the hook through sh regardless of the executable bit
}
