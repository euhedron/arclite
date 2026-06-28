use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::cli::{GlobalArgs, UpdateArgs};
use crate::output::emit;

/// arc's own GitHub workspace + repo — the single base for every release URL: the git remote
/// (releases are `v*` tags this check reads via `git ls-remote`), the human Downloads page, and the
/// Downloads REST endpoint (`--apply` pulls the per-OS binary). One home so the three can't drift.
const WORKSPACE: &str = "nikganderson";
const REPO: &str = "arclite";

/// Env var holding the GitHub credential `--apply` uses to fetch the binary: a `user:token` Basic
/// pair (e.g. `you@example.com:<api-token>`). Read from the environment so no secret lands in a file
/// arc tracks; the version check needs none (it rides the user's existing git credential).
const AUTH_ENV: &str = "ARC_GITHUB_AUTH";

/// A released version as a comparable `[major, minor, patch]` triple (arc tags are plain `vX.Y.Z`).
type Version = [u64; 3];

/// The `update` command. Without `--apply`, report whether a newer arc is published (the running
/// binary vs. the highest release tag); with `--apply`, download that release and install it in place.
/// The version check consults git over HTTPS with the credential a push already uses — no token — so it
/// works wherever git does; only the `--apply` download needs [`AUTH_ENV`].
pub fn run(args: &UpdateArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    clean_stale_backup(); // tidy a prior --apply's leftover backup, best-effort
    let current = current_version();
    let latest = latest_version()?;
    let available = latest > current;
    if args.apply {
        return apply(current, latest, available, args.force, global);
    }
    let human = if available {
        format!(
            "arc {} is out of date — {} is the latest release.\nInstall it with `arc update --apply`, or download manually from {}",
            version_string(current),
            version_string(latest),
            downloads_page(),
        )
    } else {
        format!(
            "arc {} is up to date (latest release: {}).",
            version_string(current),
            version_string(latest),
        )
    };
    let payload = serde_json::json!({
        "current": version_string(current),
        "latest": version_string(latest),
        "update_available": available,
    });
    emit(&payload, &human, global.json)
}

/// Download the target release's binary and install it over the running one. With no newer release,
/// does nothing unless `--force` (a reinstall/repair of the current version).
fn apply(
    current: Version,
    latest: Version,
    available: bool,
    force: bool,
    global: &GlobalArgs,
) -> anyhow::Result<()> {
    if !available && !force {
        let human = format!(
            "arc {} is already the latest release — nothing to apply (use --force to reinstall).",
            version_string(current)
        );
        let payload = serde_json::json!({
            "current": version_string(current),
            "latest": version_string(latest),
            "applied": false,
        });
        return emit(&payload, &human, global.json);
    }
    // --force with no newer release reinstalls the current version; otherwise install the latest.
    let target = if available { latest } else { current };
    let name = binary_name(target);
    let auth = std::env::var(AUTH_ENV).map_err(|_| {
        anyhow::anyhow!(
            "set {AUTH_ENV} to a GitHub credential (user:token, e.g. you@example.com:<api-token>) to install updates, or download {name} manually from {}",
            downloads_page()
        )
    })?;
    let exe = std::env::current_exe().context("locating the running arc binary to replace")?;
    let download_path = sidecar(&exe, ".arc-update-new");
    // Stage the download, then install it. On any failure — a partial download, or an install that
    // rolled back its own rename — the staging file may remain, so remove it on the error path (warn
    // if it can't be removed; an absent file is the normal, silent case) and propagate the error.
    if let Err(e) = download(&download_api_url(&name), &auth, &download_path)
        .and_then(|()| install(&exe, &download_path))
    {
        if let Err(rm) = std::fs::remove_file(&download_path)
            && rm.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "arclite: could not remove the staging file {} ({rm})",
                download_path.display()
            );
        }
        return Err(e);
    }
    let human = format!(
        "updated arc {} → {}. Restart arc to use the new version.",
        version_string(current),
        version_string(target),
    );
    let payload = serde_json::json!({
        "current": version_string(current),
        "latest": version_string(latest),
        "applied": true,
        "installed": version_string(target),
    });
    emit(&payload, &human, global.json)
}

// ---- version check ----

/// The running binary's version, from the compile-time package version (the single source of truth for
/// what this binary is).
fn current_version() -> Version {
    parse_version(env!("CARGO_PKG_VERSION"))
        .expect("CARGO_PKG_VERSION is always valid X.Y.Z semver")
}

/// The highest `v*` release tag in the update remote, via `git ls-remote --tags` — authenticated by
/// the user's existing git credential, exactly like a push. Tags that aren't plain `vX.Y.Z` (and the
/// `^{}` dereference lines annotated tags emit) are skipped. Errors only if git can't be consulted, so
/// a network or auth failure surfaces rather than masquerading as "up to date".
fn latest_version() -> anyhow::Result<Version> {
    let remote = format!("https://github.com/{WORKSPACE}/{REPO}.git");
    let output = crate::ai::command("git")?
        .args(["ls-remote", "--tags"])
        .arg(&remote)
        .output()
        .map_err(|e| anyhow::anyhow!("could not run git to check for updates: {e}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-remote could not reach {remote}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            // Each line is "<sha>\t<ref>"; keep refs under refs/tags/, dropping the `^{}` deref suffix.
            let reference = line.split('\t').nth(1)?;
            let tag = reference.strip_prefix("refs/tags/")?;
            parse_version(tag.strip_suffix("^{}").unwrap_or(tag))
        })
        .max()
        .ok_or_else(|| anyhow::anyhow!("no version tags found at {remote}"))
}

/// Parse `X.Y.Z` (with an optional leading `v`) into a comparable triple; `None` for anything else, so
/// a non-release tag is skipped rather than mis-parsed.
fn parse_version(s: &str) -> Option<Version> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some([major, minor, patch])
}

/// Render a version triple as `X.Y.Z` for display and JSON.
fn version_string(v: Version) -> String {
    format!("{}.{}.{}", v[0], v[1], v[2])
}

/// For the TUI's startup check: the latest release version *if* it's newer than the running binary,
/// else `None`. Best-effort — any failure (offline, git missing) yields `None` rather than nagging,
/// since this drives only an optional "update available" hint, not a user-invoked check.
pub(crate) fn newer_release() -> Option<String> {
    let latest = latest_version().ok()?;
    (latest > current_version()).then(|| version_string(latest))
}

// ---- release URLs / artifact name ----

/// The human-facing Downloads page (shown when pointing a user at a manual download).
fn downloads_page() -> String {
    format!("https://github.com/{WORKSPACE}/{REPO}/downloads")
}

/// The Downloads REST endpoint for one artifact — it accepts the API token and 302-redirects to the
/// signed storage URL (the download host), where the auth header must not follow.
fn download_api_url(name: &str) -> String {
    format!("https://api.github.com/2.0/repositories/{WORKSPACE}/{REPO}/downloads/{name}")
}

/// The release artifact name for the running platform — `arc-v<version>-<os>-<arch><exe-suffix>`. Must
/// match the names the release pipeline uploads (`github-pipelines.yml`); a mismatch surfaces as a
/// download 404 (a clear failure), never a silently wrong file.
fn binary_name(v: Version) -> String {
    format!(
        "arc-v{}-{}-{}{}",
        version_string(v),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::EXE_SUFFIX,
    )
}

// ---- download + install ----

/// Download `url` to `dest` with `curl`, authenticating with the `user:token` `auth` pair. The
/// credential is fed to curl as a config directive over **stdin** (`--config -`) — never argv (a
/// process listing would expose it) and never a temp file (whose permissions vary by platform); curl
/// drops the auth header on the cross-host redirect GitHub issues to signed storage. On Windows the
/// system `curl.exe` is used (Schannel → system/corp cert store) with revocation checks disabled, since
/// corp networks commonly block the revocation endpoints.
fn download(url: &str, auth: &str, dest: &Path) -> anyhow::Result<()> {
    // The credential becomes a curl config line; a quote or newline would break the quoting or inject
    // further directives, so require a single clean `user:token` line rather than trust the input.
    anyhow::ensure!(
        !auth.contains(['"', '\n', '\r']),
        "{AUTH_ENV} must be a single `user:token` line with no quotes or newlines"
    );
    let mut cmd = Command::new(curl_program());
    cmd.args(["--location", "--fail", "--silent", "--show-error"])
        .args(["--config", "-"]) // read the credential config from stdin — not argv, not a file
        .arg("--output")
        .arg(dest)
        .arg(url)
        .stdin(std::process::Stdio::piped());
    if cfg!(windows) {
        cmd.arg("--ssl-no-revoke");
    }
    let mut child = cmd
        .spawn()
        .context("running curl to download the update (is curl installed?)")?;
    // Hand curl the credential, then close stdin so it proceeds. The config is one short line consumed
    // before the download, so this can't deadlock against the output (which goes to `dest`, not a pipe).
    child
        .stdin
        .take()
        .expect("curl stdin was piped")
        .write_all(format!("user = \"{auth}\"\n").as_bytes())
        .context("passing the credential to curl")?;
    let status = child.wait().context("waiting for curl")?;
    anyhow::ensure!(
        status.success(),
        "curl could not download {url} (exit {}) — if this platform's release isn't published yet, download it manually from {}",
        status
            .code()
            .map_or_else(|| "signal".to_owned(), |c| c.to_string()),
        downloads_page(),
    );
    let len = std::fs::metadata(dest)
        .context("the downloaded update could not be read")?
        .len();
    anyhow::ensure!(len > 0, "the downloaded update was empty");
    Ok(())
}

/// The curl to invoke. On Windows, the system `curl.exe` specifically (so TLS goes through Schannel and
/// the system/corp cert store, not a bundled CA set a shadowing MSYS curl would use); elsewhere, `curl`
/// on `PATH`.
fn curl_program() -> PathBuf {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let system_curl = Path::new(&root).join("System32").join("curl.exe");
        // Prefer it unless *confirmed* absent: an unreadable or uncertain probe (try_exists Err) must
        // not collapse into "absent" and fall back to a possibly-shadowing PATH curl, so treat
        // can't-tell as present — keeping absent distinct from unreadable.
        if system_curl.try_exists().unwrap_or(true) {
            return system_curl;
        }
    }
    PathBuf::from("curl")
}

/// Install `new` over the running binary `exe`. On Windows a running `.exe` can't be overwritten but can
/// be renamed, so the running image is moved to a `.old` sidecar and the new binary takes its place (the
/// running process keeps the renamed image; the next launch uses the replacement; the `.old` is cleaned
/// on a later run). On Unix a rename over the path replaces it while the running process keeps the old
/// inode.
fn install(exe: &Path, new: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The exec bit is essential — a non-executable arc is broken — so surface a failed chmod with
        // context rather than swallow it, mirroring init.rs::make_executable.
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755)).with_context(
            || {
                format!(
                    "making the downloaded binary executable ({})",
                    new.display()
                )
            },
        )?;
        std::fs::rename(new, exe)
            .with_context(|| format!("installing the new binary at {}", exe.display()))
    }
    #[cfg(windows)]
    {
        let backup = sidecar(exe, ".old");
        // A Windows rename fails if the destination exists, so remove any leftover backup first —
        // surfacing a real failure (e.g. a stale backup still locked by a running old process) rather
        // than letting the later rename fail opaquely; an absent backup (the normal case) is fine.
        match std::fs::remove_file(&backup) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| {
                    format!(
                        "removing a stale backup before install ({})",
                        backup.display()
                    )
                });
            }
        }
        std::fs::rename(exe, &backup)
            .with_context(|| format!("moving the running binary aside ({})", exe.display()))?;
        match std::fs::rename(new, exe) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Roll back so an arc.exe still exists; if even that fails the install is left without
                // one, so say so loudly with the recovery path rather than swallowing it.
                if let Err(restore) = std::fs::rename(&backup, exe) {
                    eprintln!(
                        "arclite: could not restore the original binary ({restore}) — it is saved at {}; rename it back to {} manually",
                        backup.display(),
                        exe.display()
                    );
                }
                Err(e).with_context(|| format!("installing the new binary at {}", exe.display()))
            }
        }
    }
}

/// Remove a `.old` backup a prior Windows `--apply` left behind. Absent is the normal case (silent); a
/// present-but-undeletable backup (the prior binary may still be running) is benign and retried on a
/// later run, but it's surfaced — matching the codebase's warn-on-cleanup-failure standard — rather
/// than swallowed. A no-op on Unix, which replaces the binary atomically and never writes a backup.
fn clean_stale_backup() {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!(
                "arclite: could not locate the running binary to clean its update backup ({e})"
            );
            return;
        }
    };
    let backup = sidecar(&exe, ".old");
    if let Err(e) = std::fs::remove_file(&backup)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!(
            "arclite: could not remove the old binary backup {} ({e})",
            backup.display()
        );
    }
}

/// A sibling path formed by appending `suffix` to `path`'s full name (not replacing its extension, so
/// `arc.exe` + `.old` is `arc.exe.old`). Used for the download, the curl config, and the backup.
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_tags_and_rejects_non_releases() {
        assert_eq!(parse_version("v0.1.2"), Some([0, 1, 2]));
        assert_eq!(parse_version("0.1.2"), Some([0, 1, 2])); // CARGO_PKG_VERSION has no `v`
        assert_eq!(parse_version("v1.20.300"), Some([1, 20, 300]));
        assert_eq!(parse_version("v0.1"), None); // fewer than three parts
        assert_eq!(parse_version("v0.1.2.3"), None); // more than three parts
        assert_eq!(parse_version("v0.1.x"), None); // non-numeric component
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn compares_versions_numerically_not_lexically() {
        // The crux: 0.10.0 must outrank 0.9.0 — a string compare would get this backwards.
        assert!(parse_version("v0.10.0") > parse_version("v0.9.0"));
        assert!(parse_version("v0.1.2") > parse_version("v0.1.1"));
        assert!(parse_version("v1.0.0") > parse_version("v0.99.99"));
        assert_eq!(parse_version("v0.1.2"), parse_version("0.1.2")); // `v` prefix is irrelevant
    }

    #[test]
    fn compile_time_version_always_parses() {
        // current_version() unwraps this, so the package version must always be a valid triple.
        assert!(parse_version(env!("CARGO_PKG_VERSION")).is_some());
    }

    #[test]
    fn binary_name_matches_release_artifact_shape() {
        let name = binary_name([0, 1, 2]);
        assert!(name.starts_with("arc-v0.1.2-"));
        assert!(name.contains(std::env::consts::OS));
        assert!(name.contains(std::env::consts::ARCH));
        assert!(name.ends_with(std::env::consts::EXE_SUFFIX));
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert_eq!(name, "arc-v0.1.2-windows-x86_64.exe"); // the exact name published today
    }

    #[test]
    fn sidecar_appends_without_replacing_extension() {
        assert_eq!(
            sidecar(Path::new("arc.exe"), ".old"),
            PathBuf::from("arc.exe.old")
        );
        assert_eq!(
            sidecar(Path::new("/usr/bin/arc"), ".tmp"),
            PathBuf::from("/usr/bin/arc.tmp")
        );
    }

    #[test]
    fn install_puts_new_binary_in_place() {
        // Deterministic test of the swap on throwaway files — no network, no real binary touched.
        let dir = std::env::temp_dir().join(format!("arc-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(format!("arc{}", std::env::consts::EXE_SUFFIX));
        let new = sidecar(&exe, ".arc-update-new");
        std::fs::write(&exe, b"OLD").unwrap();
        std::fs::write(&new, b"NEW").unwrap();
        install(&exe, &new).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW"); // the new binary is in place
        assert!(!new.exists()); // the temp download was consumed by the rename
        #[cfg(windows)]
        assert_eq!(std::fs::read(sidecar(&exe, ".old")).unwrap(), b"OLD"); // old moved aside
        let _ = std::fs::remove_dir_all(&dir);
    }
}
