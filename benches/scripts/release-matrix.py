#!/usr/bin/env python3
"""Run the local, cross-volume, PipeTransport, and SSH release matrix."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--initial-destination", type=Path)
    parser.add_argument("--workload", default="initial-copy")
    parser.add_argument("--same-volume", type=Path, required=True)
    parser.add_argument("--cross-volume", type=Path)
    parser.add_argument("--ssh-host")
    parser.add_argument("--ssh-destination", default="/tmp/xsync-release-matrix")
    parser.add_argument("--remote-binary")
    parser.add_argument("--ssh-filesystem", default="unknown")
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--xsync", type=Path, default=Path("target/release/xsync"))
    parser.add_argument("--bench", type=Path, default=Path("target/release/xsync-bench"))
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    return parser.parse_args()


def time_command(command: list[str]) -> tuple[subprocess.CompletedProcess[str], float, int]:
    started = time.monotonic()
    completed = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    elapsed = time.monotonic() - started
    rss = 0
    for line in completed.stderr.splitlines():
        if "maximum resident set size" in line:
            rss = int(line.split()[0])
    return completed, elapsed, rss


def finished(stdout: str) -> dict:
    for line in stdout.splitlines():
        value = json.loads(line)
        if value.get("event") == "finished":
            return value
    raise RuntimeError("xsync output did not contain a finished event")


def seed_destination(args: argparse.Namespace, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)
    if args.initial_destination:
        shutil.copytree(args.initial_destination, destination, dirs_exist_ok=True, symlinks=True)
        for source_path in sorted(args.initial_destination.rglob("*")):
            relative = source_path.relative_to(args.initial_destination)
            target_path = destination / relative
            if target_path.exists() or target_path.is_symlink():
                shutil.copystat(source_path, target_path, follow_symlinks=False)
        shutil.copystat(args.initial_destination, destination, follow_symlinks=False)


def run_row(args: argparse.Namespace, route: str, destination: Path, raw: bool) -> dict:
    seed_destination(args, destination)
    command = [str(args.xsync), "--progress-json"]
    if args.workload in {"delete", "interrupted-resume"}:
        command.append("--delete")
    if args.workload == "content-churn":
        command.append("--checksum")
    if raw:
        command.append("--no-compress")
    if route == "pipe":
        wrapper = destination.parent / "xsync-pipe-rsh.sh"
        wrapper.write_text(
            "#!/bin/sh\n"
            "shift\n"
            "eval \"set -- $1\"\n"
            f"exec '{args.xsync.resolve()}' \"$2\" \"$3\"\n",
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        command.extend(["--transport=xsync", "-e", str(wrapper)])
        command.extend([f"{platform.node()}:{destination}"])
        source_arg = f"{args.source}/"
    else:
        source_arg = f"{args.source}/"
        command.extend([source_arg, f"{destination}/"])
    if route == "pipe":
        command.insert(-1, source_arg)
    completed, seconds, rss = time_command(command)
    if completed.returncode:
        raise RuntimeError(
            f"{route} failed ({completed.returncode}): {completed.stderr[-4000:]}"
        )
    result = finished(completed.stdout)
    verify = subprocess.run(
        [str(args.bench), "verify", str(destination), "--manifest", str(args.manifest)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if verify.returncode:
        raise RuntimeError(f"{route} correctness failed: {verify.stderr}")
    return {
        "route": route,
        "workload": args.workload,
        "compression": (
            "none (local route)"
            if route == "local"
            else ("none" if raw else "adaptive-zstd")
        ),
        "repetition": None,
        "wall_seconds": seconds,
        "peak_rss_bytes": rss,
        "logical_bytes": result.get("transferred_bytes", 0),
        "wire_bytes": result.get("wire_bytes", 0),
        "transferred_files": result.get("transferred_files", 0),
        "correctness": "passed",
    }


def blocked(route: str, reason: str) -> dict:
    return {"route": route, "status": "blocked", "reason": reason}


def attempt_row(args: argparse.Namespace, route: str, destination: Path, raw: bool, repetition: int) -> dict:
    try:
        row = run_row(args, "pipe" if route == "pipe" else "local", destination, raw)
        row.update({"route": route, "repetition": repetition})
        return row
    except (OSError, RuntimeError) as error:
        return {
            "route": route,
            "workload": args.workload,
            "repetition": repetition,
            "status": "failed",
            "reason": str(error),
        }


def run_ssh_matrix(args: argparse.Namespace, output_root: Path) -> dict:
    if not args.ssh_host:
        return {"status": "blocked", "reason": "--ssh-host was not supplied"}
    if not args.remote_binary:
        return {"status": "blocked", "reason": "--remote-binary was not supplied"}
    remote_json = output_root / "ssh-matrix.json"
    remote_markdown = output_root / "ssh-matrix.md"
    command = [
        "python3",
        str(Path(__file__).with_name("remote-matrix.py")),
        "--source", str(args.source),
        "--manifest", str(args.manifest),
        "--host", args.ssh_host,
        "--remote-binary", args.remote_binary,
        "--destination-base", args.ssh_destination,
        "--filesystem", args.ssh_filesystem,
        "--profile", "release",
        "--repetitions", str(args.repetitions),
        "--json", str(remote_json),
        "--markdown", str(remote_markdown),
    ]
    completed = subprocess.run(command, text=True, capture_output=True)
    if completed.returncode:
        raise RuntimeError(f"ssh matrix failed:\n{completed.stdout}\n{completed.stderr}")
    return {"status": "passed", "report": json.loads(remote_json.read_text())}


def run_production_rsync(args: argparse.Namespace, output_root: Path) -> list[dict]:
    if not args.ssh_host or not args.remote_binary:
        return []
    rows = []
    for repetition in range(args.repetitions):
        target = f"{args.ssh_destination}-production-rsync-{repetition}"
        subprocess.run(
            ["ssh", "-o", "BatchMode=yes", args.ssh_host, "rm", "-rf", target],
            check=True, text=True, capture_output=True,
        )
        command = [
            str(args.xsync), "--progress-json", "--transport", "rsync",
            f"{args.source}/", f"{args.ssh_host}:{target}/",
        ]
        completed, seconds, rss = time_command(command)
        if completed.returncode:
            raise RuntimeError(f"production rsync failed: {completed.stderr[-4000:]}")
        result = finished(completed.stdout)
        verify = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", args.ssh_host, args.remote_binary,
             "manifest", "--root", target],
            check=True, text=True, capture_output=True,
        )
        actual = json.loads(verify.stdout)
        expected = json.loads(args.manifest.read_text())
        if (actual["manifest_digest"], actual["item_count"], actual["logical_bytes"]) != (
            expected["manifest_digest"], expected["item_count"], expected["logical_bytes"]
        ):
            raise RuntimeError("production rsync correctness failed")
        rows.append({
            "route": "production-rsync",
            "workload": args.workload,
            "repetition": repetition,
            "compression": "unsupported by rsync transport",
            "wall_seconds": seconds,
            "peak_rss_bytes": rss,
            "logical_bytes": result.get("transferred_bytes", 0),
            "wire_bytes": result.get("wire_bytes", 0),
            "transferred_files": result.get("transferred_files", 0),
            "correctness": "passed",
        })
    return rows


def main() -> int:
    args = parse_args()
    if args.repetitions < 5:
        raise SystemExit("--repetitions must be at least 5 for a release gate")
    rows: list[dict] = []
    for route, root in [("same-volume", args.same_volume), ("cross-volume", args.cross_volume)]:
        if root is None:
            rows.append(blocked(route, "no destination supplied"))
            continue
        for raw in (False, True):
            for repetition in range(args.repetitions):
                destination = root / f"release-matrix-{route}-{'raw' if raw else 'zstd'}-{repetition}"
                if destination.exists():
                    shutil.rmtree(destination)
                rows.append(attempt_row(args, route, destination, raw, repetition))
    for raw in (False, True):
        for repetition in range(args.repetitions):
            destination = args.same_volume / f"release-matrix-pipe-{'raw' if raw else 'zstd'}-{repetition}"
            if destination.exists():
                shutil.rmtree(destination)
            rows.append(attempt_row(args, "pipe", destination, raw, repetition))
    rows.extend(run_production_rsync(args, args.json.parent))
    ssh_report = run_ssh_matrix(args, args.json.parent)
    if ssh_report["status"] == "blocked":
        rows.append(blocked("ssh", ssh_report["reason"]))
    report = {
        "schema": "xsync.release-matrix.v1",
        "host": {"os": platform.platform(), "machine": platform.machine()},
        "source": str(args.source),
        "manifest": str(args.manifest),
        "workload": args.workload,
        "repetitions": args.repetitions,
        "rows": rows,
        "ssh_matrix": ssh_report,
    }
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    lines = ["# xsync Release Matrix", "", "| Route | Compression | Repetition | Wall seconds | Wire bytes | Correctness |", "|---|---|---:|---:|---:|---|"]
    for row in rows:
        if row.get("status") in {"blocked", "failed"}:
            lines.append(f"| {row['route']} | {row['status']} | {row.get('repetition', '-')} | - | - | {row['reason']} |")
        else:
            lines.append(f"| {row['route']} | {row['compression']} | {row['repetition']} | {row['wall_seconds']:.4f} | {row['wire_bytes']} | {row['correctness']} |")
    if ssh_report["status"] == "passed":
        lines.extend([
            "",
            "## SSH Matrix",
            "",
            "The complete SSH method table is embedded in `ssh-matrix.json` and `ssh-matrix.md`.",
        ])
    else:
        lines.append(f"| ssh | blocked | - | - | - | {ssh_report['reason']} |")
    args.markdown.parent.mkdir(parents=True, exist_ok=True)
    args.markdown.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(args.json)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
