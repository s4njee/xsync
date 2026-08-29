#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'usage: %s <host> [amd64|arm64] [remote-path]\n' "$0" >&2
    exit 2
}

[[ $# -ge 1 && $# -le 3 ]] || usage
host=$1
architecture=${2:-}
remote_path=${3:-.local/bin/xs}
root=$(cd "$(dirname "$0")/.." && pwd)

if [[ -z "$architecture" ]]; then
    remote_arch=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" uname -m)
    case "$remote_arch" in
        x86_64|amd64) architecture=amd64 ;;
        aarch64|arm64) architecture=arm64 ;;
        *) printf 'unsupported remote architecture %s on %s\n' "$remote_arch" "$host" >&2; exit 1 ;;
    esac
fi

case "$architecture" in
    amd64) target=x86_64-unknown-linux-gnu ;;
    arm64) target=aarch64-unknown-linux-gnu ;;
    *) usage ;;
esac

(cd "$root" && cargo zigbuild --release --target "$target" -p xsync)
binary="$root/target/$target/release/xs"
local_sha=$(shasum -a 256 "$binary" | awk '{print $1}')
remote_sha=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
    "sha256sum '$remote_path' 2>/dev/null || shasum -a 256 '$remote_path' 2>/dev/null || true" \
    | awk 'NR == 1 {print $1}')

if [[ "$remote_sha" == "$local_sha" ]]; then
    printf 'agent current at %s:%s (%s)\n' "$host" "$remote_path" "$local_sha"
    exit 0
fi

if [[ -n "$remote_sha" ]]; then
    printf 'agent stale at %s:%s (remote %s, required %s); restaging\n' \
        "$host" "$remote_path" "$remote_sha" "$local_sha"
else
    printf 'agent missing at %s:%s; staging\n' "$host" "$remote_path"
fi
"$root/scripts/stage-linux.sh" "$host" "$remote_path" "$architecture"

installed_sha=$(ssh "$host" \
    "sha256sum '$remote_path' 2>/dev/null || shasum -a 256 '$remote_path'" | awk 'NR == 1 {print $1}')
[[ "$installed_sha" == "$local_sha" ]] || {
    printf 'agent restage verification failed: expected %s, got %s\n' "$local_sha" "$installed_sha" >&2
    exit 1
}
