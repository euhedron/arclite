//! Minimal HTTP GET via the system `curl` — the one statement of arclite's outbound-HTTP mechanics,
//! shared by the self-updater and the provider model listings. Secret header values ride curl's stdin
//! config (`--config -`) — never argv (a process listing would expose them) and never a temp file;
//! plain headers (an Accept, a version pin) go on argv normally.
//!
//! Redirects are per-call, because curl's cross-host credential stripping covers only its
//! *authentication* credentials (`Authorization`, per `--location`'s "curl only sends its credentials
//! to the initial host") — a custom secret header (an `x-api-key`) is forwarded to whatever host a
//! redirect names. So a caller sending Authorization to an endpoint whose contract *is* a redirect
//! (a release asset's 302 to signed storage) opts in; one sending a custom-header credential must
//! not follow redirects at all — refused outright rather than trusted to a stripping that never runs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;

/// `GET url`, returning the response body. `plain` headers go on argv; `secret` header values go via
/// stdin config directives. `follow_redirects` is the module-doc contract: on for an
/// Authorization-credentialed endpoint whose designed flow is a redirect, off — fail closed — for a
/// custom-header credential curl would forward. With `dest`, the body is written there (a binary) and
/// the returned string is empty; without, the body is captured and returned (JSON metadata). A
/// `User-Agent` is always sent (some APIs — GitHub's — reject requests without one).
pub(crate) fn get(
    url: &str,
    plain: &[(&str, &str)],
    secret: &[(&str, &str)],
    follow_redirects: bool,
    dest: Option<&Path>,
) -> anyhow::Result<String> {
    for (name, value) in secret {
        // A secret becomes a curl config line; a quote, backslash (curl's escape char), or newline
        // would break the quoting or inject further directives, so require a clean single-line value.
        anyhow::ensure!(
            !value.contains(['"', '\\', '\n', '\r']),
            "the {name} credential must be a single line with no quotes, backslashes, or newlines"
        );
    }
    let mut cmd = Command::new(curl_program()?);
    // stderr is captured, never inherited: `--show-error`'s diagnostic belongs in the returned
    // error, and a caller may hold the terminal exclusively (the TUI's model fetch) — a child
    // writing there directly would corrupt the display.
    cmd.args(["--fail", "--silent", "--show-error"])
        .args(["--user-agent", "arclite"])
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if follow_redirects {
        cmd.arg("--location");
    }
    for (name, value) in plain {
        cmd.arg("--header").arg(format!("{name}: {value}"));
    }
    if let Some(dest) = dest {
        cmd.arg("--output").arg(dest);
    }
    if !secret.is_empty() {
        // Read the credential headers from config directives on stdin — not argv, not a file.
        cmd.args(["--config", "-"]).stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().context("running curl (is curl installed?)")?;
    if !secret.is_empty() {
        // Hand curl the headers, then close stdin (dropping the handle) so it proceeds. A few short
        // lines, consumed before any response body, so this can't deadlock against captured stdout.
        let mut stdin = child.stdin.take().expect("curl stdin was piped");
        for (name, value) in secret {
            stdin
                .write_all(format!("header = \"{name}: {value}\"\n").as_bytes())
                .context("passing a credential to curl")?;
        }
    }
    let output = child.wait_with_output().context("waiting for curl")?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason.trim();
        anyhow::bail!(
            "curl could not GET {url} (exit {}){}{}",
            output
                .status
                .code()
                .map_or_else(|| "signal".to_owned(), |c| c.to_string()),
            if reason.is_empty() { "" } else { ": " },
            reason,
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The curl to invoke. On Windows, the system `curl.exe` specifically (so TLS goes through Schannel and
/// the system cert store, not a bundled CA set a shadowing MSYS curl would use); elsewhere, `curl` on
/// `PATH`.
fn curl_program() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    {
        // The trusted binary resolves via SystemRoot; with it unset the explicit path can't be built
        // at all, and a bare PATH `curl` could be any shadowing binary — so that anomaly errors
        // rather than silently downgrading the trust anchor.
        let root = std::env::var_os("SystemRoot").ok_or_else(|| {
            anyhow::anyhow!(
                "SystemRoot is unset, so the system curl.exe can't be located — refusing to fall \
                 back to an arbitrary curl on PATH"
            )
        })?;
        let system_curl = Path::new(&root).join("System32").join("curl.exe");
        // Prefer it unless *confirmed* absent: an unreadable or uncertain probe (try_exists Err) must
        // not collapse into "absent" and fall back to a possibly-shadowing PATH curl, so treat
        // can't-tell as present — keeping absent distinct from unreadable. Confirmed absence (an older
        // Windows that ships no curl) is the one case PATH serves, and it's this disclosed line.
        if system_curl.try_exists().unwrap_or(true) {
            return Ok(system_curl);
        }
    }
    Ok(PathBuf::from("curl"))
}
