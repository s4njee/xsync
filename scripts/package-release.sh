#!/usr/bin/env bash
# Build and package one release artifact. Story D2.2.
#
# Deliberately a script rather than inline CI steps: a release process that only
# exists inside a workflow cannot be run, inspected, or debugged without pushing
# a tag, which is exactly when you least want to be debugging it.
#
#   scripts/package-release.sh <target-triple> [output-dir]
set -euo pipefail

target=${1:?usage: package-release.sh <target-triple> [output-dir]}
outdir=${2:-dist}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ -z "$version" ]; then
    printf 'cannot read version from Cargo.toml\n' >&2
    exit 1
fi

# Pin the build date so two runs from the same commit agree. Without this the
# timestamp alone makes binaries differ, which defeats the reproducibility the
# story asks for. Prefer the commit's own time so the value is a property of the
# source rather than of when the build happened to run.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct 2>/dev/null || date -u +%s)
    export SOURCE_DATE_EPOCH
fi

printf 'building %s (version %s, SOURCE_DATE_EPOCH=%s)\n' "$target" "$version" "$SOURCE_DATE_EPOCH"
# `cross` for targets this machine cannot build natively; plain cargo otherwise.
${XSYNC_CARGO:-cargo} build --release --locked --target "$target" -p xsync

case "$target" in
    *windows*) binary=xs.exe ;;
    *)         binary=xs ;;
esac
built="target/$target/release/$binary"
[ -f "$built" ] || { printf 'expected binary missing: %s\n' "$built" >&2; exit 1; }

stage="$outdir/xsync-$version-$target"
rm -rf "$stage"
mkdir -p "$stage"
cp "$built" "$stage/$binary"
cp LICENSE-MIT LICENSE-APACHE "$stage/"
cp README.md "$stage/README.md"

mkdir -p "$outdir"

# Normalise mtimes before archiving. GNU tar's --mtime/--sort/--owner are not
# accepted by the bsdtar macOS ships, and the first version of this script fell
# through to a plain `tar -czf` there, producing archives that differed between
# runs even though the binaries inside them were byte-identical. Setting the
# times on disk works with either tar.
stamp=$(TZ=UTC date -r "$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S 2>/dev/null \
     || TZ=UTC date -u -d "@$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S)
find "$stage" -exec touch -t "$stamp" {} +

case "$target" in
    *windows*)
        archive="$outdir/xsync-$version-$target.zip"
        rm -f "$archive"
        # -X drops extra attributes that vary per machine; the normalised
        # mtimes above supply the rest of the determinism.
        (cd "$outdir" && find "$(basename "$stage")" -type f | LC_ALL=C sort | zip -qX "$(basename "$archive")" -@)
        ;;
    *)
        archive="$outdir/xsync-$version-$target.tar.gz"
        rm -f "$archive"
        # Files only: naming the directory as well made tar recurse into it and
        # archive every member twice, doubling the artifact while still hashing
        # identically between runs. Reproducible and wrong is still wrong.
        # Explicit sorted member list rather than --sort, and `gzip -n` so the
        # compressor does not stamp its own timestamp into the header.
        (cd "$outdir" \
            && find "$(basename "$stage")" -type f -print0 | LC_ALL=C sort -z \
            | tar --null -T - --uid 0 --gid 0 --numeric-owner -cf -) \
            | gzip -n -9 > "$archive"
        ;;
esac

rm -rf "$stage"
printf 'packaged %s (%s bytes)\n' "$archive" "$(wc -c < "$archive" | tr -d ' ')"
