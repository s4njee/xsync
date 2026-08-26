#!/usr/bin/env bash
# Build .deb and .rpm from a staged release tree. Story D4.3.
#
#   scripts/package-linux.sh <target-triple> [outdir]
#
# Uses nfpm, which builds both formats from one declarative spec and needs
# neither dpkg nor rpmbuild, so the packages can be produced from any host
# rather than only from the distribution they target.
set -euo pipefail

target=${1:?usage: package-linux.sh <target-triple> [outdir]}
outdir=${2:-dist}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

case "$target" in
    *-linux-*) ;;
    *) printf 'package-linux.sh is for Linux targets, got %s\n' "$target" >&2; exit 1 ;;
esac

command -v nfpm >/dev/null 2>&1 || {
    printf 'nfpm not found. Install: go install github.com/goreleaser/nfpm/v2/cmd/nfpm@latest\n' >&2
    exit 1
}

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
case "$target" in
    x86_64-*)  arch=amd64 ;;
    aarch64-*) arch=arm64 ;;
    *) printf 'unsupported architecture in %s\n' "$target" >&2; exit 1 ;;
esac

stage="$outdir/linux-$target"
rm -rf "$stage"; mkdir -p "$stage" "$outdir"

binary="target/$target/release/xs"
[ -f "$binary" ] || { printf 'build first: %s missing\n' "$binary" >&2; exit 1; }
cp "$binary" "$stage/xs"

# Generated from the parser, so a package can never document flags that the
# binary it contains does not have.
"$stage/xs" --man > "$stage/xs.1" 2>/dev/null || cargo run -q -p xsync --bin xs -- --man > "$stage/xs.1"
for shell in bash zsh fish; do
    case "$shell" in
        bash) out="xs" ;;
        zsh)  out="_xs" ;;
        fish) out="xs.fish" ;;
    esac
    "$stage/xs" --completions "$shell" > "$stage/completion-$out" 2>/dev/null \
        || cargo run -q -p xsync --bin xs -- --completions "$shell" > "$stage/completion-$out"
done

# The glibc floor is a package-level declaration so installation fails cleanly
# on an old distribution, rather than the binary failing at runtime with a
# loader error that says nothing useful. Floor comes from docs/TARGET-MATRIX.md.
case "$target" in
    *-musl) depends="" ;;
    *)      depends="libc6 >= 2.28" ;;
esac

# nfpm resolves `src` against the working directory rather than the config
# file, so the spec is written with absolute paths instead of relative ones
# that would silently depend on where the script was invoked from.
abs_stage=$(cd "$stage" && pwd)

cat > "$stage/nfpm.yaml" <<YAML
name: xsync
arch: $arch
platform: linux
version: "$version"
section: utils
priority: optional
maintainer: xsync contributors <noreply@github.com>
description: |
  High-performance rsync replacement built on a parallel pipeline and BLAKE3.
vendor: xsync
homepage: https://github.com/s4njee/xsync
license: MIT OR Apache-2.0
contents:
  - src: $abs_stage/xs
    dst: /usr/bin/xs
    file_info: { mode: 0755 }
  - src: $abs_stage/xs.1
    dst: /usr/share/man/man1/xs.1
    file_info: { mode: 0644 }
  - src: $abs_stage/completion-xs
    dst: /usr/share/bash-completion/completions/xs
    file_info: { mode: 0644 }
  - src: $abs_stage/completion-_xs
    dst: /usr/share/zsh/site-functions/_xs
    file_info: { mode: 0644 }
  - src: $abs_stage/completion-xs.fish
    dst: /usr/share/fish/vendor_completions.d/xs.fish
    file_info: { mode: 0644 }
  - src: $root/LICENSE-MIT
    dst: /usr/share/doc/xsync/LICENSE-MIT
    file_info: { mode: 0644 }
  - src: $root/LICENSE-APACHE
    dst: /usr/share/doc/xsync/LICENSE-APACHE
    file_info: { mode: 0644 }
YAML

if [ -n "$depends" ]; then
    printf 'overrides:\n  deb:\n    depends:\n      - "%s"\n' "$depends" >> "$stage/nfpm.yaml"
fi

for format in deb rpm; do
    nfpm package --config "$stage/nfpm.yaml" --packager "$format" --target "$outdir/"
done

rm -rf "$stage"
printf 'packaged Linux artifacts into %s\n' "$outdir"
