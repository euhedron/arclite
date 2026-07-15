use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

use crate::cli::{GlobalArgs, UpdateArgs};
use crate::output::emit;

/// arc's own GitHub owner + repo — the single base for every release URL: the git remote (releases are
/// `v*` tags this check reads via `git ls-remote`), the human Releases page, and the release-asset URL
/// (`--apply` pulls the per-OS binary). One home so the three can't drift.
const OWNER: &str = "nikganderson";
const REPO: &str = "arclite";

/// The GitHub web/git host and the REST API host — single-sourced alongside OWNER/REPO so the URL
/// builders below can't drift on scheme or host. The web host serves the human Releases page and the
/// git remote (the version check); the API host serves the asset lookup + download `--apply` uses.
const HOST: &str = "https://github.com";
const API_HOST: &str = "https://api.github.com";

/// Env var holding an optional GitHub token `--apply` uses to fetch the binary. A public release needs
/// none; a private repo's assets need a token (a fine-grained `contents:read`, or a classic PAT). Read
/// from the environment so no secret lands in a file arc tracks; the version check needs none either
/// (it rides the user's existing git credential).
const AUTH_ENV: &str = "ARC_GITHUB_TOKEN";

/// A released version as a comparable `[major, minor, patch]` triple (arc tags are plain `vX.Y.Z`).
type Version = [u64; 3];

/// The `update` command. Without `--apply`, report whether a newer arc is published (the running
/// binary vs. the highest release tag); with `--apply`, download that release and install it in place.
/// The version check consults git over HTTPS with the credential a push already uses — no token — so it
/// works wherever git does; only the `--apply` download needs [`AUTH_ENV`].
pub fn run(args: &UpdateArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    // Tidy a prior --apply's leftover backup, best-effort. Windows-only in fact as well as in doc:
    // only the Windows install path ever writes the `.old` sidecar, and an unconditional remove on
    // Unix could delete a same-named sibling file the updater never created.
    #[cfg(windows)]
    clean_stale_backup();
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
            releases_page(),
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
    // A public release needs no credential; a private repo needs a token for both the asset lookup and
    // the download. Optional — sent to curl only when set, so the public case stays frictionless.
    let auth = std::env::var(AUTH_ENV).ok();
    let exe = std::env::current_exe().context("locating the running arc binary to replace")?;
    let download_path = sidecar(&exe, ".arc-update-new");
    // Resolve this platform's asset id from the releases API, then download it by id — the documented
    // way to fetch an asset's bytes (there is no download-by-name endpoint), working for public and
    // private repos alike. The lookup precedes any staging, so a failure here leaves nothing to clean up.
    let asset_id = release_asset_id(target, &name, auth.as_deref())?;
    // Claim the staging name atomically (exclusive create) before curl writes into it: two concurrent
    // `arc update --apply` runs share this fixed sidecar path, and without the claim one could truncate
    // the file mid-download while the other installs it — a corrupt binary swapped into place. The
    // loser fails fast here instead; a leftover claim from a hard-killed run is named so it can be
    // removed (every non-crash path below already cleans it up).
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&download_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                anyhow::anyhow!(
                    "another arc update appears to be in progress ({} exists) — if none is, remove that file and retry",
                    download_path.display()
                )
            } else {
                anyhow::Error::new(e)
                    .context(format!("cannot stage the update at {}", download_path.display()))
            }
        })?;
    // Stage the download, verify the staged binary *is* the version its release claims (run its
    // --version and compare — the tag named the target, but only the binary's own report confirms
    // what was actually built), then install it. On any failure — a partial download, a version
    // mismatch, or an install that rolled back its own rename — the staging file may remain, so
    // remove it on the error path (warn if it can't be removed; an absent file is the normal,
    // silent case) and propagate the error.
    if let Err(e) = download(
        &asset_download_url(asset_id),
        auth.as_deref(),
        &download_path,
    )
    .and_then(|()| verify_staged_version(&download_path, target))
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
    let remote = format!("{HOST}/{OWNER}/{REPO}.git");
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

/// The human-facing Releases page (shown when pointing a user at a manual download).
fn releases_page() -> String {
    format!("{HOST}/{OWNER}/{REPO}/releases")
}

/// The REST API URL for a release asset's *bytes*: a `GET` with `Accept: application/octet-stream`
/// 302-redirects to signed storage (curl drops the auth header on that cross-host hop, which carries
/// its own auth). The numeric `asset_id` comes from [`release_asset_id`] — the API has no
/// download-by-name endpoint, so the id must be resolved first.
fn asset_download_url(asset_id: u64) -> String {
    format!("{API_HOST}/repos/{OWNER}/{REPO}/releases/assets/{asset_id}")
}

/// Resolve this platform's binary (`name`) to its numeric asset id in the `v<version>` release, via the
/// releases API (`GET /releases/tags/{tag}` → its `assets` array). A private repo needs a token for this
/// call too; a public one needs none. The two failure modes read distinctly — couldn't reach/authorize
/// the release (network/auth, hinting at the token when none was given) vs. the release carries no asset
/// for this platform — so a not-yet-published build isn't a mystery.
fn release_asset_id(target: Version, name: &str, auth: Option<&str>) -> anyhow::Result<u64> {
    #[derive(Deserialize)]
    struct Asset {
        id: u64,
        name: String,
    }
    #[derive(Deserialize)]
    struct Release {
        assets: Vec<Asset>,
    }
    let url = format!(
        "{API_HOST}/repos/{OWNER}/{REPO}/releases/tags/v{}",
        version_string(target)
    );
    let body = curl_get(&url, auth, "application/vnd.github+json", None).with_context(|| {
        let hint = if auth.is_none() {
            format!(" (a private repo needs {AUTH_ENV} set)")
        } else {
            String::new()
        };
        format!(
            "looking up the v{} release on GitHub{hint}",
            version_string(target)
        )
    })?;
    let release: Release =
        serde_json::from_str(&body).context("parsing the GitHub release metadata")?;
    release
        .assets
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| a.id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the v{} release has no {name} asset — this platform's binary may not be published yet; download manually from {}",
                version_string(target),
                releases_page(),
            )
        })
}

/// The release-asset naming convention, stated once: `arc-<tag>-<os>-<arch><exe-suffix>` (the tag
/// carries its `v`). [`binary_name`] instantiates it for the running platform, and the
/// `release_workflow_assets_follow_the_one_naming_convention` test holds the release workflow's
/// hand-written matrix to the same format — the enforcement behind "these must match".
fn asset_name(tag: &str, os: &str, arch: &str, exe_suffix: &str) -> String {
    format!("arc-{tag}-{os}-{arch}{exe_suffix}")
}

/// The release artifact name for the running platform. A mismatch against the published assets
/// surfaces as a download 404 (a clear failure), never a silently wrong file.
fn binary_name(v: Version) -> String {
    asset_name(
        &format!("v{}", version_string(v)),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::EXE_SUFFIX,
    )
}

// ---- download + install ----

/// Download the release asset at `url` to `dest`. Thin wrapper over [`curl_get`] with the
/// `application/octet-stream` Accept that makes the API return the binary bytes, plus a non-empty check.
fn download(url: &str, auth: Option<&str>, dest: &Path) -> anyhow::Result<()> {
    curl_get(url, auth, "application/octet-stream", Some(dest)).with_context(|| {
        format!(
            "downloading the update (if this platform's release isn't published, download it manually from {})",
            releases_page()
        )
    })?;
    let len = std::fs::metadata(dest)
        .context("the downloaded update could not be read")?
        .len();
    anyhow::ensure!(len > 0, "the downloaded update was empty");
    Ok(())
}

/// `GET url` via [`crate::http::get`] — the shared curl path (secrets on stdin, never argv) — with
/// `auth` (if set) as a `Bearer` credential. `accept` sets the Accept header; with `dest`, the body
/// lands there (a binary), else it's returned (JSON metadata).
fn curl_get(
    url: &str,
    auth: Option<&str>,
    accept: &str,
    dest: Option<&Path>,
) -> anyhow::Result<String> {
    let bearer = auth.map(|token| format!("Bearer {token}"));
    let secret: Vec<(&str, &str)> = bearer
        .as_deref()
        .map(|b| ("Authorization", b))
        .into_iter()
        .collect();
    // Redirects on: the release-asset flow *is* a 302 to signed storage. The credential is
    // Authorization-class, and the client's cross-host stripping is confirmed, not assumed: curl
    // removes an Authorization header — including one set via a header directive, this path — on any
    // cross-host redirect (behavior fixed in curl 7.58, CVE-2018-1000007), so the token reaches only
    // the initial API host and never rides the hop to storage. The acknowledged-hop contract lives on
    // http::get's `follow_redirects` parameter (see the module doc).
    crate::http::get(url, &[("Accept", accept)], &secret, true, dest)
}

/// Confirm the staged binary reports the version its release tag promised, before it replaces the
/// running arc: run it with `--version` and require the target version in its output. The tag chose
/// what to download; only the binary's own report confirms what was built — `installed` must never
/// claim a version the binary doesn't. (Executing the staged file here is not an added trust step:
/// install would make it *the* arc one rename later.)
fn verify_staged_version(staged: &Path, target: Version) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("making the staged binary runnable ({})", staged.display()))?;
    }
    let output = std::process::Command::new(staged)
        .arg("--version")
        .output()
        .with_context(|| format!("running the staged binary ({})", staged.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "the staged binary's --version exited with {}",
        output.status
    );
    let reported = String::from_utf8_lossy(&output.stdout);
    let expected = version_string(target);
    anyhow::ensure!(
        reported.split_whitespace().any(|w| w == expected),
        "the staged binary reports `{}`, not the release's v{expected} — refusing to install a mislabeled binary",
        reported.trim()
    );
    Ok(())
}

/// Suffix of the backup the Windows swap leaves beside the exe — named once so [`install`] (which
/// writes it) and [`clean_stale_backup`] (which must delete exactly that name) cannot disagree.
/// Windows-only like its two users: no Unix code may ever touch the name.
#[cfg(windows)]
const BACKUP_SUFFIX: &str = ".old";

/// Install `new` over the running binary `exe`. On Windows a running `.exe` can't be overwritten but
/// can be renamed, so the running image is moved to a [`BACKUP_SUFFIX`] sidecar and the new binary
/// takes its place (the running process keeps the renamed image; the next launch uses the
/// replacement; the backup is cleaned on a later run). On Unix a rename over the path replaces it
/// while the running process keeps the old inode.
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
        let backup = sidecar(exe, BACKUP_SUFFIX);
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

/// Remove a [`BACKUP_SUFFIX`] backup a prior Windows `--apply` left behind. Absent is the normal case (silent); a
/// present-but-undeletable backup (the prior binary may still be running) is benign and retried on a
/// later run, but it's surfaced — matching the codebase's warn-on-cleanup-failure standard — rather
/// than swallowed. Compiled (and called) on Windows only: Unix replaces the binary atomically, never
/// writes a backup, and must not delete a same-named file it didn't create.
#[cfg(windows)]
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
    let backup = sidecar(&exe, BACKUP_SUFFIX);
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
/// `arc.exe` + `.old` is `arc.exe.old`). Used for the download staging file and the backup.
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

    /// The workflow's hand-written asset matrix can't be generated from [`asset_name`] (YAML can't
    /// call it), so this test is the enforcement that keeps the two homes one: it parses the
    /// workflow's YAML and reads the actual matrix rows — fields, not raw text a comment could
    /// satisfy — asserting each platform's `asset` is exactly what `asset_name` produces for its
    /// target, and that the platform sets match, so adding to one side without the other fails
    /// here — not as a download 404 months later.
    #[test]
    fn release_workflow_assets_follow_the_one_naming_convention() {
        let yml_text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.github/workflows/release.yml"
        ))
        .expect("the release workflow exists at its documented path");
        let docs =
            yaml_rust2::YamlLoader::load_from_str(&yml_text).expect("release.yml parses as YAML");
        let rows = docs[0]["jobs"]["upload"]["strategy"]["matrix"]["include"]
            .as_vec()
            .expect("the upload matrix is a list of platform rows");
        // Each published Rust target's (os, arch, exe-suffix) — the values std::env::consts
        // reports on that platform, which binary_name uses at run time.
        let platform = |target: &str| match target {
            "x86_64-unknown-linux-gnu" => ("linux", "x86_64", ""),
            "aarch64-apple-darwin" => ("macos", "aarch64", ""),
            "x86_64-apple-darwin" => ("macos", "x86_64", ""),
            "x86_64-pc-windows-msvc" => ("windows", "x86_64", ".exe"),
            other => panic!("release.yml publishes {other}, which this test doesn't pin — add it"),
        };
        let tag = "${{ github.ref_name }}"; // as the workflow interpolates it into `asset`
        assert_eq!(
            rows.len(),
            4,
            "a platform was added or removed — update this test's mapping"
        );
        for row in rows {
            let target = row["target"].as_str().expect("a matrix row has a target");
            let asset = row["asset"].as_str().expect("a matrix row has an asset");
            let (os, arch, exe) = platform(target);
            assert_eq!(
                asset,
                asset_name(tag, os, arch, exe),
                "the {target} asset drifted from asset_name()"
            );
        }
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
        assert_eq!(std::fs::read(sidecar(&exe, BACKUP_SUFFIX)).unwrap(), b"OLD"); // old moved aside
        let _ = std::fs::remove_dir_all(&dir);
    }
}
