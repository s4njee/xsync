#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'usage: %s <host> <remote-path> <amd64|arm64>\n' "$0" >&2
    exit 2
}

[[ $# -eq 3 ]] || usage

host=$1
remote_path=$2
architecture=$3

case "$architecture" in
    amd64) target=x86_64-unknown-linux-gnu ;;
    arm64) target=aarch64-unknown-linux-gnu ;;
    *) usage ;;
esac

command -v cargo-zigbuild >/dev/null || {
    printf 'cargo-zigbuild is required; install it on the macOS build machine\n' >&2
    exit 1
}
command -v zig >/dev/null || {
    printf 'zig is required by cargo-zigbuild; install it on the macOS build machine\n' >&2
    exit 1
}
command -v ssh >/dev/null || {
    printf 'ssh is required\n' >&2
    exit 1
}
command -v shasum >/dev/null || {
    printf 'shasum is required\n' >&2
    exit 1
}

root=$(cd "$(dirname "$0")/.." && pwd)
binary="$root/target/$target/release/xs"

printf 'building xs for %s (%s)\n' "$architecture" "$target"
(cd "$root" && cargo zigbuild --release --target "$target" -p xsync)

[[ -x "$binary" ]] || {
    printf 'build did not produce executable %s\n' "$binary" >&2
    exit 1
}

local_sha=$(shasum -a 256 "$binary" | awk '{print $1}')
remote_dir=${remote_path%/*}
remote_base=${remote_path##*/}
[[ "$remote_dir" != "$remote_path" ]] || remote_dir=.
remote_tmp="$remote_dir/.xsync-stage.$remote_base.$$.tmp"

shell_quote() {
    local value=$1
    value=${value//\'/\'\\\'\'}
    printf "'%s'" "$value"
}

cleanup() {
    ssh "$host" "rm -f $(shell_quote "$remote_tmp")" >/dev/null 2>&1 || true
}
trap cleanup EXIT

printf 'uploading verified temporary binary to %s:%s\n' "$host" "$remote_tmp"
ssh "$host" "mkdir -p $(shell_quote "$remote_dir") && rm -f $(shell_quote "$remote_tmp")"
scp "$binary" "$host:$remote_tmp"

remote_sha=$(ssh "$host" "sha256sum $(shell_quote "$remote_tmp") 2>/dev/null || shasum -a 256 $(shell_quote "$remote_tmp")" | awk '{print $1}')
[[ "$remote_sha" == "$local_sha" ]] || {
    printf 'remote checksum mismatch: local %s, remote %s\n' "$local_sha" "$remote_sha" >&2
    exit 1
}

ssh "$host" "chmod 755 $(shell_quote "$remote_tmp") && mv -f $(shell_quote "$remote_tmp") $(shell_quote "$remote_path")"
trap - EXIT

final_sha=$(ssh "$host" "sha256sum $(shell_quote "$remote_path") 2>/dev/null || shasum -a 256 $(shell_quote "$remote_path")" | awk '{print $1}')
[[ "$final_sha" == "$local_sha" ]] || {
    printf 'installed checksum mismatch: local %s, remote %s\n' "$local_sha" "$final_sha" >&2
    exit 1
}

printf 'staged %s at %s:%s (%s)\n' "$target" "$host" "$remote_path" "$local_sha"
