#!/bin/zsh

# Capture syscall-count summaries for T1.1 on macOS.
#
# Usage:
#   sudo ./benches/scripts/t1-syscall-trace.sh SOURCE [OUTPUT_DIR]
#
# SOURCE must be a corpus directory. The script creates fresh disposable
# destinations below OUTPUT_DIR and never writes inside SOURCE.

set -euo pipefail

script_dir="${0:A:h}"
repo_root="${script_dir:h:h}"
xsync_bin="${XSYNC_BIN:-$repo_root/target/release/xsync}"
output_dir="${2:-$repo_root/benches/results/tuning/T1/syscall-trace-$(date +%Y%m%d-%H%M%S)}"
source_arg="${1:-}"

die() {
  print -u2 -- "error: $*"
  exit 1
}

[[ $EUID -eq 0 ]] || die "run this script with sudo"
[[ -n "$source_arg" ]] || die "usage: sudo $0 SOURCE [OUTPUT_DIR]"
[[ -d "$source_arg" ]] || die "source is not a directory: $source_arg"
[[ -x "$xsync_bin" ]] || die "xsync binary is not executable: $xsync_bin"
command -v dtruss >/dev/null || die "dtruss is not available"
command -v rsync >/dev/null || die "rsync is not available"

source_dir="$(cd "$source_arg" && pwd -P)"
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd -P)"

[[ "$output_dir/" != "$source_dir/"* ]] || die "output directory is inside the source corpus"

run_dir="$output_dir/run"
mkdir -p "$run_dir"
[[ -z "$(find "$run_dir" -mindepth 1 -maxdepth 1 -print -quit)" ]] \
  || die "output directory already contains a run; choose a new OUTPUT_DIR"

xsync_dest="$run_dir/xsync-destination"
rsync_dest="$run_dir/rsync-destination"
mkdir -p "$xsync_dest" "$rsync_dest"

print -- "source: $source_dir"
print -- "output: $output_dir"
print -- "xsync:  $xsync_bin"
print -- "running dtruss for xsync..."

{
  print -- "tool=xsync"
  print -- "command=$xsync_bin --progress-json $source_dir/ $xsync_dest/"
  print -- "started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  dtruss -c -f "$xsync_bin" --progress-json "$source_dir/" "$xsync_dest/" \
    > /dev/null
} > "$output_dir/xsync-command.txt" 2> "$output_dir/xsync-dtruss.txt"

print -- "running dtruss for rsync..."

{
  print -- "tool=rsync-a"
  print -- "command=rsync -a $source_dir/ $rsync_dest/"
  print -- "started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  dtruss -c -f rsync -a "$source_dir/" "$rsync_dest/" \
    > /dev/null
} > "$output_dir/rsync-command.txt" 2> "$output_dir/rsync-dtruss.txt"

{
  print -- "captured=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  print -- "macos=$(sw_vers -productVersion)"
  print -- "kernel=$(uname -a)"
  print -- "source=$source_dir"
  print -- "xsync=$xsync_bin"
  print -- "xsync_sha256=$(shasum -a 256 "$xsync_bin" | awk '{print $1}')"
  print -- "rsync=$(rsync --version | sed -n '1p')"
} > "$output_dir/environment.txt"

print -- "trace complete"
print -- "provide these files:"
print -- "  $output_dir/environment.txt"
print -- "  $output_dir/xsync-dtruss.txt"
print -- "  $output_dir/rsync-dtruss.txt"
