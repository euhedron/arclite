# arc installer (Windows) — https://github.com/euhedron/arclite
#
#   irm https://raw.githubusercontent.com/euhedron/arclite/main/install.ps1 | iex
#
# Downloads the windows-x86_64 binary from the repo's GitHub Releases, verifies it against the
# release's SHA256SUMS when that asset exists (releases before v0.1.12 predate it), installs it as
# arc.exe under %LOCALAPPDATA%\Programs\arc, and adds that directory to the user PATH if absent.
# Script installs self-update thereafter via `arc update --apply`.
$ErrorActionPreference = "Stop"

$repo = "euhedron/arclite"
$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$tag = $release.tag_name
# The release-asset naming convention (update.rs::asset_name is its home): arc-<tag>-<os>-<arch>.
$asset = "arc-$tag-windows-x86_64.exe"
$url = "https://github.com/$repo/releases/download/$tag/$asset"

$dir = Join-Path $env:LOCALAPPDATA "Programs\arc"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$tmp = Join-Path $dir "arc.exe.download"
Write-Host "downloading $asset ..."
Invoke-WebRequest -Uri $url -OutFile $tmp

$sums = $release.assets | Where-Object { $_.name -eq "SHA256SUMS" }
if ($sums) {
    $line = (Invoke-RestMethod $sums.browser_download_url) -split "`n" |
        Where-Object { ($_ -split '\s+')[-1] -eq $asset } | Select-Object -First 1
    $want = if ($line) { ($line -split '\s+')[0].ToLower() } else { $null }
    $got = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLower()
    if (-not $want -or $want -ne $got) {
        Remove-Item $tmp
        throw "arc install: checksum mismatch for $asset (expected '$want', got '$got')"
    }
    Write-Host "checksum verified."
} else {
    Write-Host "note: $tag publishes no SHA256SUMS (releases before v0.1.12) - skipping checksum verification."
}

$dest = Join-Path $dir "arc.exe"
Move-Item -Force $tmp $dest
Write-Host "installed: $(& $dest --version) -> $dest"

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dir", "User")
    Write-Host "added $dir to your user PATH - restart the terminal to pick it up."
}
