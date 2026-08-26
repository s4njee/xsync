#!/usr/bin/env bash
# Print the CHANGELOG section for one version, for use as release notes.
#
# Notes are derived from the changelog rather than kept in a second file,
# because two files describing one release drift, and the one that drifts is
# always the one nobody reads until after publishing.
set -euo pipefail

version=${1:?usage: release-notes.sh <version>}
root=$(cd "$(dirname "$0")/.." && pwd)
changelog="$root/CHANGELOG.md"

# Print from this version's heading up to (not including) the next one.
section=$(awk -v v="$version" '
    $0 ~ "^## \\[?" v "\\]?" { found = 1; next }
    found && /^## / { exit }
    found { print }
' "$changelog")

if [ -z "$(printf '%s' "$section" | tr -d '[:space:]')" ]; then
    printf 'no CHANGELOG.md section for version %s\n' "$version" >&2
    exit 1
fi

printf '%s\n' "$section"
