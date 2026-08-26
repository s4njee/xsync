#!/usr/bin/env bash
set -euo pipefail

# Verify the three external host classes from Story 13.2 without stopping at
# the first unavailable host. Use --stage after verification to publish the
# already-built target through stage-linux.sh.

usage() {
    printf 'usage: %s [--stage]\n' "$0" >&2
    exit 2
}

stage=0
while (($#)); do
    case "$1" in
        --stage) stage=1 ;;
        *) usage ;;
    esac
    shift
done

root=$(cd "$(dirname "$0")/.." && pwd)
zfs_host=${XSYNC_ZFS_HOST:-freya.local}
ext23_host=${XSYNC_EXT23_HOST:-192.168.1.119}
arm64_host=${XSYNC_ARM64_HOST:-gentoo-rpi5.local}
remote_path=${XSYNC_REMOTE_PATH:-/tmp/xsync-stage/xs}

printf 'Story 13.2 cross-builds\n'
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    if cargo zigbuild --release --target "$target" -p xsync; then
        artifact="$root/target/$target/release/xs"
        sha=$(shasum -a 256 "$artifact" | awk '{print $1}')
        printf '  PASS build %-30s %s\n' "$target" "$sha"
    else
        printf '  BLOCKED build %-28s cargo zigbuild failed\n' "$target"
    fi
done

check_host() {
    local label=$1 host=$2 expected_arch=$3 expected_fs=$4 architecture=$5
    local facts
    if ! facts=$(ssh -o BatchMode=yes -o ConnectTimeout=4 "$host" \
        'printf "%s %s %s" "$(uname -s)" "$(uname -m)" "$(stat -f -c %T "$HOME" 2>/dev/null || true)"' 2>&1); then
        printf '  BLOCKED %-12s %s: %s\n' "$label" "$host" "$facts"
        return 0
    fi
    local os arch fs
    read -r os arch fs <<<"$facts"
    if [[ "$os" != Linux || "$arch" != "$expected_arch" ]]; then
        printf '  BLOCKED %-12s %s: expected Linux/%s, got %s/%s\n' "$label" "$host" "$expected_arch" "$os" "$arch"
        return 0
    fi
    if [[ "$fs" != "$expected_fs" ]]; then
        printf '  BLOCKED %-12s %s: expected filesystem %s, got %s\n' "$label" "$host" "$expected_fs" "$fs"
        return 0
    fi
    printf '  PASS %-16s %s (%s/%s)\n' "$label" "$host" "$arch" "$fs"
    if ((stage)); then
        scripts/stage-linux.sh "$host" "$remote_path" "$architecture"
    fi
}

check_host 'x86_64 ZFS' "$zfs_host" x86_64 zfs amd64
check_host 'x86_64 ext2/3' "$ext23_host" x86_64 ext2/ext3 amd64
check_host 'aarch64 ext4' "$arm64_host" aarch64 ext4 arm64

printf 'Unavailable hosts are reported as blockers; verification continues for every host.\n'
