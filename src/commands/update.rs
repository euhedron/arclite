use crate::cli::{GlobalArgs, UpdateArgs};
use crate::output::emit;

/// arc's own repository on GitHub — the single base URL for both the git remote (releases are `v*`
/// tags this check reads via `git ls-remote`) and the Downloads page (per-OS binaries a later
/// `--apply` will pull). One home so the check and the apply can't drift to different repos.
const UPDATE_REPO_URL: &str = "https://github.com/nikganderson/arclite";

/// A released version as a comparable `[major, minor, patch]` triple (arc tags are plain `vX.Y.Z`).
type Version = [u64; 3];

/// The `update` command: compare the running binary's version against the highest released tag and
/// report whether a newer arc is published. The check consults git over HTTPS with the credential a
/// push already uses — no token handling — so it works wherever `git push` to the repo does.
pub fn run(_args: &UpdateArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let current = current_version();
    let latest = latest_version()?;
    let available = latest > current;
    let human = if available {
        format!(
            "arc {} is out of date — {} is the latest release.\nDownload: {UPDATE_REPO_URL}/downloads",
            version_string(current),
            version_string(latest),
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
    let remote = format!("{UPDATE_REPO_URL}.git");
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
}
