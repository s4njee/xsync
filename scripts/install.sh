#!/usr/bin/env sh
# xsync installer. Story D4.1.
#
#   curl -fsSL https://raw.githubusercontent.com/s4njee/xsync/main/scripts/install.sh | sh
#
# Installs to a user-local prefix, so it never needs root and never writes
# outside the invoking user's home unless told to.
#
#   XSYNC_VERSION   version to install (default: latest release)
#   XSYNC_BIN_DIR   install directory (default: $HOME/.local/bin)
#
# POSIX sh rather than bash: this is the one script that must run on whatever
# shell a stranger's machine happens to have.
set -eu

REPO=${XSYNC_REPO:-s4njee/xsync}
BIN_DIR=${XSYNC_BIN_DIR:-"$HOME/.local/bin"}

say()  { printf '%s\n' "$*"; }
fail() { printf 'install: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

# --- detect the platform -----------------------------------------------------
detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Darwin) os_part="apple-darwin" ;;
        Linux)
            # musl and glibc binaries are not interchangeable; picking wrong
            # produces a loader error that explains nothing.
            if (ldd --version 2>&1 || true) | grep -qi musl; then
                os_part="unknown-linux-musl"
            else
                os_part="unknown-linux-gnu"
            fi
            ;;
        *) fail "unsupported operating system: $os (see docs/TARGET-MATRIX.md)" ;;
    esac
    case "$arch" in
        x86_64|amd64)  arch_part="x86_64" ;;
        aarch64|arm64) arch_part="aarch64" ;;
        *) fail "unsupported architecture: $arch (xsync builds for x86_64 and aarch64)" ;;
    esac
    printf '%s-%s' "$arch_part" "$os_part"
}

# --- resolve the version -----------------------------------------------------
latest_version() {
    # Ask the API rather than following /releases/latest, so a repository with
    # no releases yet fails with a clear message instead of a 404 page.
    curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' \
        | head -1
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        fail "no sha256 tool found (need sha256sum or shasum); refusing to install unverified"
    fi
}

need curl
need tar

target=$(detect_target)
version=${XSYNC_VERSION:-$(latest_version)}
[ -n "$version" ] || fail "cannot determine the latest version of $REPO; set XSYNC_VERSION to install a specific one"

archive="xsync-$version-$target.tar.gz"
base="https://github.com/$REPO/releases/download/v$version"

say "xsync $version"
say "  platform:  $target"
say "  installing to: $BIN_DIR"

tmp=$(mktemp -d)
# Clean up on every exit path, including interruption: a failed install must
# not leave a half-downloaded binary anywhere.
trap 'rm -rf "$tmp"' EXIT INT TERM

say "  downloading $archive"
curl -fsSL "$base/$archive" -o "$tmp/$archive" \
    || fail "download failed: $base/$archive"

# --- verify before touching anything ----------------------------------------
say "  verifying checksum"
if curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" 2>/dev/null; then
    expected=$(sed -n "s/^\([0-9a-f]\{64\}\)[[:space:]]*[.\/]*$archive$/\1/p" "$tmp/SHA256SUMS" | head -1)
    if [ -z "$expected" ]; then
        fail "SHA256SUMS has no entry for $archive; refusing to install unverified.
  Verify manually: https://github.com/$REPO/blob/main/docs/verifying-downloads.md"
    fi
    actual=$(sha256_of "$tmp/$archive")
    if [ "$expected" != "$actual" ]; then
        fail "checksum mismatch for $archive
  expected: $expected
  actual:   $actual
This is not a transient failure. Do not install this file.
  Re-download and check again, and if it persists report it at
  https://github.com/$REPO/issues"
    fi
    say "  checksum ok"
else
    fail "cannot fetch SHA256SUMS from $base; refusing to install unverified.
  To proceed manually see https://github.com/$REPO/blob/main/docs/verifying-downloads.md"
fi

# --- install -----------------------------------------------------------------
tar -xzf "$tmp/$archive" -C "$tmp"
binary=$(find "$tmp" -type f -name xs -perm -u+x 2>/dev/null | head -1)
[ -n "$binary" ] || fail "archive did not contain an xs binary"

mkdir -p "$BIN_DIR"
# Install via a temporary name in the destination directory, then rename: a
# rename is atomic within a filesystem, so an interrupted install cannot leave a
# truncated binary where a working one used to be.
install_tmp="$BIN_DIR/.xs.install.$$"
cp "$binary" "$install_tmp"
chmod 755 "$install_tmp"
mv -f "$install_tmp" "$BIN_DIR/xs"

say ""
say "installed: $BIN_DIR/xs"
"$BIN_DIR/xs" -V 2>/dev/null || true

# Say plainly whether it is usable, rather than assuming the prefix is on PATH.
case ":${PATH}:" in
    *":$BIN_DIR:"*) say "$BIN_DIR is on your PATH; run: xs --help" ;;
    *)
        say ""
        say "$BIN_DIR is NOT on your PATH. Add it:"
        say "  export PATH=\"\$PATH:$BIN_DIR\""
        say "and put that line in your shell profile."
        ;;
esac
