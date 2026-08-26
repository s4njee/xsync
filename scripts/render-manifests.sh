#!/usr/bin/env bash
# Fill the packaging templates from a release's SHA256SUMS. Story D4.2/D4.4.
#
#   scripts/render-manifests.sh <version> <path-to-SHA256SUMS> [outdir]
#
# Rendered from the published checksums rather than recomputed, so a manifest
# can only ever describe artifacts that were actually released.
set -euo pipefail

version=${1:?usage: render-manifests.sh <version> <SHA256SUMS> [outdir]}
sums=${2:?usage: render-manifests.sh <version> <SHA256SUMS> [outdir]}
outdir=${3:-dist/manifests}
root=$(cd "$(dirname "$0")/.." && pwd)

[ -f "$sums" ] || { printf 'no such checksum file: %s\n' "$sums" >&2; exit 1; }
mkdir -p "$outdir"

# Look up one artifact's digest by target triple.
digest_for() {
    triple=$1
    ext=$2
    line=$(grep -E "[[:space:]][.\/]*xsync-${version}-${triple}\.${ext}$" "$sums" | head -1 || true)
    if [ -z "$line" ]; then
        printf '' && return 0
    fi
    printf '%s' "${line%% *}"
}

render() {
    template=$1
    output=$2
    content=$(cat "$template")
    content=${content//__VERSION__/$version}
    missing=""
    for spec in \
        "aarch64-apple-darwin:tar.gz" \
        "x86_64-apple-darwin:tar.gz" \
        "aarch64-unknown-linux-gnu:tar.gz" \
        "x86_64-unknown-linux-gnu:tar.gz" \
        "x86_64-pc-windows-msvc:zip" \
        "aarch64-pc-windows-msvc:zip"
    do
        triple=${spec%%:*}
        ext=${spec##*:}
        token="__SHA256_$(printf '%s' "$triple" | tr 'a-z-' 'A-Z_')__"
        case "$content" in
            *"$token"*) ;;
            *) continue ;;
        esac
        sha=$(digest_for "$triple" "$ext")
        if [ -z "$sha" ]; then
            missing="$missing $triple"
            continue
        fi
        content=${content//$token/$sha}
    done
    if [ -n "$missing" ]; then
        # A manifest with an unfilled placeholder installs nothing and fails
        # confusingly at the user's machine. Refuse here instead.
        printf 'cannot render %s: no checksum for:%s\n' "$(basename "$template")" "$missing" >&2
        return 1
    fi
    printf '%s\n' "$content" > "$output"
    printf 'rendered %s\n' "$output"
}

render "$root/packaging/homebrew/xsync.rb"  "$outdir/xsync.rb"
render "$root/packaging/scoop/xsync.json"   "$outdir/xsync.json"
