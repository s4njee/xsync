#!/usr/bin/env bash
set -euo pipefail

# Story 14.3 entry point. It runs both projects' conformance suites before
# checking whether the checked-out f2 client has the full mutation/transfer
# surface required by the joint smoke.

usage() {
    printf 'usage: %s [--f2-root PATH]\n' "$0" >&2
    exit 2
}

f2_root=${F2_ROOT:-"$(cd "$(dirname "$0")/../.." && pwd)/f2"}
while (($#)); do
    case "$1" in
        --f2-root)
            (($# >= 2)) || usage
            f2_root=$2
            shift
            ;;
        *) usage ;;
    esac
    shift
done

root=$(cd "$(dirname "$0")/.." && pwd)
[[ -f "$f2_root/Package.swift" ]] || {
    printf 'BLOCKED [f2] checkout not found: %s\n' "$f2_root" >&2
    exit 3
}

printf '[xsync] v2 codec and server tests\n'
(cd "$root" && cargo test -p xsync-core protocol_v2)

printf '[f2] shared-vector and browse codec tests\n'
if ! (cd "$f2_root" && swift test --filter F2ProtocolTests); then
    printf 'FAILED [f2] protocol suite; inspect the preceding f2 output\n' >&2
    exit 1
fi

missing=()
for symbol in renameRequest createDirectoryRequest deleteRequest fetchRequest publishRequest; do
    if ! rg -q "case $symbol" "$f2_root/Sources/F2Protocol"; then
        missing+=("$symbol")
    fi
done
if ((${#missing[@]})); then
    printf 'BLOCKED [f2] full joint smoke is not runnable: f2 is missing v2 operations: %s\n' "${missing[*]}" >&2
    printf 'The current f2 client proves shared types 14-21, but does not yet provide the real-client steps fetch, publish, mutate, and disconnect.\n' >&2
    exit 3
fi

printf 'The full f2 client surface is present; invoke the host-backed runner from the f2 checkout before release.\n'
