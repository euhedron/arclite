---
id: the-http-path-relies-on-curls-security-behavior
status: resolved
origin_kind: system_run
system_run_id: 1784081993-17405-610330000
commit: cc6690c
recorded: 2026-07-15 02:19 UTC
---

## Claim
- **location:** src/http.rs:curl_program
- **reason:** The HTTP path relies on curl's security behavior for redirect credential handling, but non-Windows builds invoke bare `curl` from `PATH`, allowing a shadowed or incompatible implementation where the code depends on specific behavior.
- **rule:** resolve-trusted-tool-by-explicit-path

## Evidence
Promoted from `arc run audit` run `1784081993-17405-610330000` against commit `cc6690c` — see `arc log 1784081993-17405-610330000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784082568-25953-890425000`: resolved: `http::curl_program` now prefers `/usr/bin/curl` on non-Windows and `%SystemRoot%\System32\curl.exe` on Windows, falling back to PATH only when the canonical system curl is confirmed absent.
