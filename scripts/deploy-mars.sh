#!/usr/bin/env bash
set -euo pipefail

host=${1:-mars.local}
architecture=${3:-amd64}

command -v ssh >/dev/null || {
    printf 'ssh is required\n' >&2
    exit 1
}

remote_home=$(ssh "$host" 'printf %s "$HOME"')
remote_path=${2:-"$remote_home/.local/bin/xs"}

script_dir=$(cd "$(dirname "$0")" && pwd)
exec "$script_dir/stage-linux.sh" "$host" "$remote_path" "$architecture"
