---
id: the-release-workflow-never-verifies-that-the
status: resolved
origin_kind: system_run
system_run_id: 1784124280-47780-629894000
commit: 820a32c
recorded: 2026-07-15 14:04 UTC
---

## Claim
- **location:** .github/workflows/release.yml:12-14,27-39,83-89; src/commands/update.rs:90-150
- **reason:** The release workflow never verifies that the version tag matches `Cargo.toml`, and the updater reports the tag-derived target as `installed` without checking the downloaded binary's embedded version.
- **rule:** report-the-identity-that-ran

## Evidence
Promoted from `arc run audit` run `1784124280-47780-629894000` against commit `820a32c` — see `arc log 1784124280-47780-629894000` for the full run and its note.

## Why It Matters

## Next Action

## Resolution
Resolved per verify run `1784127749-81905-297967000`: The release workflow now requires the tag to match Cargo.toml, and the updater runs the staged binary with `--version` and verifies the reported target before installation.
