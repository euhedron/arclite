#!/bin/sh
# arc installer — https://github.com/euhedron/arclite
#
#   curl -fsSL https://raw.githubusercontent.com/euhedron/arclite/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/euhedron/arclite/main/install.sh | sh -s 0.1.11
#   ARC_INSTALL_DIR=~/bin sh install.sh
#
# Downloads this platform's binary from the repo's GitHub Releases, verifies it against the
# release's SHA256SUMS when that asset exists (releases before v0.1.12 predate it), and installs
# it as `arc` (default destination: ~/.local/bin). Script installs self-update thereafter via
# `arc update --apply`.
set -eu

REPO="euhedron/arclite"
DIR="${ARC_INSTALL_DIR:-$HOME/.local/bin}"

os=$(uname -s)
arch=$(uname -m)
case "$os" in
Darwin) os=macos ;;
Linux) os=linux ;;
*)
    echo "arc install: unsupported OS '$os' — Windows uses install.ps1; anything else builds from source (https://github.com/$REPO#getting-started)" >&2
    exit 1
    ;;
esac
case "$arch" in
arm64 | aarch64) arch=aarch64 ;;
x86_64 | amd64) arch=x86_64 ;;
*)
    echo "arc install: no published binary for architecture '$arch' — build from source (https://github.com/$REPO#getting-started)" >&2
    exit 1
    ;;
esac

if [ "${1:-latest}" = latest ]; then
    # The tag rides the /releases/latest redirect — no API call, no rate limit to hit.
    tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")
    tag=${tag##*/}
    case "$tag" in
    v*) ;;
    *)
        echo "arc install: could not resolve the latest release tag (got '$tag')" >&2
        exit 1
        ;;
    esac
else
    tag=$1
    case "$tag" in v*) ;; *) tag="v$tag" ;; esac
fi

# The release-asset naming convention (update.rs::asset_name is its home): arc-<tag>-<os>-<arch>.
asset="arc-$tag-$os-$arch"
url="https://github.com/$REPO/releases/download/$tag/$asset"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "downloading $asset ..."
curl -fsSL -o "$tmp/arc" "$url" || {
    echo "arc install: download failed — $tag may not publish a $os-$arch binary (see https://github.com/$REPO/releases)" >&2
    exit 1
}

if curl -fsSL -o "$tmp/SHA256SUMS" "https://github.com/$REPO/releases/download/$tag/SHA256SUMS" 2>/dev/null; then
    want=$(awk -v name="$asset" '$2 == name { print $1 }' "$tmp/SHA256SUMS")
    if command -v sha256sum >/dev/null 2>&1; then
        got=$(sha256sum "$tmp/arc" | awk '{print $1}')
    else
        got=$(shasum -a 256 "$tmp/arc" | awk '{print $1}')
    fi
    if [ -z "$want" ] || [ "$want" != "$got" ]; then
        echo "arc install: checksum mismatch for $asset (expected '${want:-none listed}', got '$got') — aborting" >&2
        exit 1
    fi
    echo "checksum verified."
else
    echo "note: $tag publishes no SHA256SUMS (releases before v0.1.12) — skipping checksum verification."
fi

mkdir -p "$DIR"
chmod 755 "$tmp/arc"
mv "$tmp/arc" "$DIR/arc"
echo "installed: $("$DIR/arc" --version) -> $DIR/arc"

case ":$PATH:" in
*":$DIR:"*) ;;
*)
    echo
    echo "$DIR is not on your PATH — add it, e.g.:"
    echo "  export PATH=\"$DIR:\$PATH\""
    ;;
esac
