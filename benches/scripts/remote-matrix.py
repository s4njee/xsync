#!/usr/bin/env python3
"""Story 0.5 real-SSH matrix for the bounded xsync framing spike."""

from __future__ import annotations

import argparse
import json
import os
import re
import resource
import statistics
import subprocess
import threading
import time
from pathlib import Path

SCHEMA = "xsync.remote-bench.report.v1"
READY = "XSYNC_RSYNC_SERVER_READY"


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--source", type=Path, required=True)
    result.add_argument("--manifest", type=Path, required=True)
    result.add_argument("--host", required=True)
    result.add_argument("--remote-binary", required=True)
    result.add_argument("--destination-base", required=True)
    result.add_argument("--filesystem", required=True)
    result.add_argument("--profile", required=True)
    result.add_argument("--receiver-prefix", default="")
    result.add_argument("--sample-bytes", type=int, default=64 * 1024)
    result.add_argument("--methods", default="rsync-a,rsync-az,xsync-1,xsync-2,xsync-4,xsync-8,xsync-adaptive-1")
    result.add_argument("--repetitions", type=int, default=5)
    result.add_argument("--json", type=Path, required=True)
    result.add_argument("--markdown", type=Path, required=True)
    return result


def run_capture(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def ssh(host: str, *arguments: str) -> list[str]:
    return ["ssh", "-o", "BatchMode=yes", host, *arguments]


def tool_version(command: list[str]) -> str:
    completed = run_capture(command)
    return completed.stdout.splitlines()[0].strip()


def cpu_usage() -> tuple[float, float]:
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime, usage.ru_stime


def run_process(command: list[str], ready_marker: str | None = None) -> dict:
    user_before, system_before = cpu_usage()
    started = time.monotonic()
    process = subprocess.Popen(
        ["/usr/bin/time", "-l", *command],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    stderr_lines: list[str] = []
    ready_seconds: list[float] = []

    def consume_stderr() -> None:
        assert process.stderr is not None
        for line in process.stderr:
            if ready_marker and ready_marker in line and not ready_seconds:
                ready_seconds.append(time.monotonic() - started)
            else:
                stderr_lines.append(line)

    reader = threading.Thread(target=consume_stderr, daemon=True)
    reader.start()
    assert process.stdout is not None
    stdout = process.stdout.read()
    status = process.wait()
    reader.join()
    elapsed = time.monotonic() - started
    user_after, system_after = cpu_usage()
    if status != 0:
        raise RuntimeError(f"command failed ({status}): {' '.join(command)}\n{''.join(stderr_lines)}")
    stderr = "".join(stderr_lines)
    rss_match = re.search(r"^\s*([0-9]+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
    return {
        "stdout": stdout,
        "stderr": stderr,
        "seconds": elapsed,
        "setup_seconds": ready_seconds[0] if ready_seconds else None,
        "cpu_seconds": (user_after - user_before) + (system_after - system_before),
        "peak_rss_bytes": int(rss_match.group(1)) if rss_match else 0,
    }


def verify_remote(args: argparse.Namespace, target: str, expected: dict) -> tuple[dict, float]:
    started = time.monotonic()
    completed = run_capture(ssh(args.host, args.remote_binary, "manifest", "--root", target))
    elapsed = time.monotonic() - started
    actual = json.loads(completed.stdout)
    passed = (
        actual["manifest_digest"] == expected["manifest_digest"]
        and actual["item_count"] == expected["item_count"]
        and actual["logical_bytes"] == expected["logical_bytes"]
    )
    return {
        "passed": passed,
        "expected_manifest_digest": expected["manifest_digest"],
        "actual_manifest_digest": actual["manifest_digest"],
        "item_count": actual["item_count"],
        "logical_bytes": actual["logical_bytes"],
    }, elapsed


def native_method(args: argparse.Namespace, method: str, target: str, expected: dict) -> dict:
    match = re.fullmatch(r"xsync-(?:(adaptive)-)?(\d+)", method)
    if not match:
        raise ValueError(f"invalid native method {method}")
    adaptive, streams = match.groups()
    command = [
        str(Path("target/release/xsync-remote-spike")),
        "send",
        "--source", str(args.source),
        "--host", args.host,
        "--remote-binary", args.remote_binary,
        "--destination", target,
        "--streams", streams,
        "--compression", "adaptive" if adaptive else "none",
        "--sample-bytes", str(args.sample_bytes),
    ]
    if args.receiver_prefix:
        command.extend(["--receiver-prefix", args.receiver_prefix])
    wall_started = time.monotonic()
    process = run_process(command)
    send = json.loads(process["stdout"])
    oracle, verification_seconds = verify_remote(args, target, expected)
    return {
        "wall_seconds": time.monotonic() - wall_started,
        "cpu_seconds": process["cpu_seconds"],
        "peak_rss_bytes": process["peak_rss_bytes"],
        "item_count": expected["item_count"],
        "logical_bytes": expected["logical_bytes"],
        "wire_bytes": send["wire_bytes"],
        "phases_seconds": {
            "ssh_setup": send["setup_seconds"],
            "transfer": send["transfer_seconds"],
            "teardown": send["teardown_seconds"],
            "independent_verification": verification_seconds,
        },
        "oracle": oracle,
        "native": send,
    }


def parse_rsync_wire(stdout: str) -> int:
    match = re.search(r"Total bytes sent:\s*([0-9,]+)", stdout)
    if not match:
        raise RuntimeError(f"rsync --stats did not report total bytes sent:\n{stdout}")
    return int(match.group(1).replace(",", ""))


def rsync_method(args: argparse.Namespace, method: str, target: str, expected: dict) -> dict:
    flags = "-az" if method == "rsync-az" else "-a"
    wrapper = f"{args.remote_binary} rsync-wrapper"
    if args.receiver_prefix:
        wrapper = f"{args.receiver_prefix} {wrapper}"
    command = [
        "rsync", flags, "--stats", "--out-format=%n", "-e", "ssh -o BatchMode=yes",
        "--rsync-path", wrapper, f"{args.source}/", f"{args.host}:{target}/",
    ]
    wall_started = time.monotonic()
    process = run_process(command, READY)
    oracle, verification_seconds = verify_remote(args, target, expected)
    setup_seconds = process["setup_seconds"]
    if setup_seconds is None:
        raise RuntimeError("remote rsync wrapper did not emit its setup marker")
    return {
        "wall_seconds": time.monotonic() - wall_started,
        "cpu_seconds": process["cpu_seconds"],
        "peak_rss_bytes": process["peak_rss_bytes"],
        "item_count": expected["item_count"],
        "logical_bytes": expected["logical_bytes"],
        "wire_bytes": parse_rsync_wire(process["stdout"]),
        "phases_seconds": {
            "ssh_setup": setup_seconds,
            "transfer_and_teardown": max(0.0, process["seconds"] - setup_seconds),
            "independent_verification": verification_seconds,
        },
        "oracle": oracle,
        "rsync_stdout": process["stdout"],
    }


def baseline_for(method: str) -> str | None:
    if method == "rsync-a":
        return None
    if method == "rsync-az":
        return "rsync-a"
    if method == "xsync-1":
        return "rsync-a"
    if method == "xsync-adaptive-1":
        return "rsync-az"
    if re.fullmatch(r"xsync-(2|4|8)", method):
        return "xsync-1"
    return None


def method_config(method: str, args: argparse.Namespace) -> dict:
    if method.startswith("rsync"):
        return {
            "transport": "reference-rsync-client",
            "streams": 1,
            "compression": "rsync -z negotiated default" if method == "rsync-az" else "none",
            "baseline": baseline_for(method),
        }
    adaptive = "adaptive" in method
    streams = int(method.rsplit("-", 1)[1])
    return {
        "transport": "native-xsync-framing-spike",
        "streams": streams,
        "compression": f"adaptive-zstd-3 sample={args.sample_bytes}" if adaptive else "none",
        "baseline": baseline_for(method),
    }


def median(values: list[float]) -> float:
    return statistics.median(values)


def mad(values: list[float]) -> float:
    center = median(values)
    return median([abs(value - center) for value in values])


def summarize(methods: list[dict]) -> None:
    by_name = {method["name"]: method for method in methods}
    for method in methods:
        walls = [sample["wall_seconds"] for sample in method["samples"]]
        wires = [sample["wire_bytes"] for sample in method["samples"]]
        summary = {
            "median_wall_seconds": median(walls),
            "mad_wall_seconds": mad(walls),
            "median_wire_bytes": int(median(wires)),
            "median_setup_seconds": median([sample["phases_seconds"]["ssh_setup"] for sample in method["samples"]]),
            "peak_rss_bytes": max(sample["peak_rss_bytes"] for sample in method["samples"]),
        }
        baseline = method["config"]["baseline"]
        if baseline:
            baseline_by_rep = {sample["repetition"]: sample for sample in by_name[baseline]["samples"]}
            ratios = [baseline_by_rep[sample["repetition"]]["wall_seconds"] / sample["wall_seconds"] for sample in method["samples"]]
            summary["paired_speedup"] = median(ratios)
            summary["paired_speedup_mad"] = mad(ratios)
        method["summary"] = summary


def schedule(methods: list[str], repetition: int) -> list[str]:
    offset = repetition % len(methods)
    rotated = methods[offset:] + methods[:offset]
    return list(reversed(rotated)) if repetition % 2 else rotated


def markdown(report: dict) -> str:
    lines = [
        "# xsync remote framing spike",
        "",
        f"- Schema: `{report['schema']}`",
        f"- Host/filesystem/profile: `{report['environment']['remote_host']}` / `{report['environment']['destination_filesystem']}` / `{report['environment']['profile']}`",
        f"- Corpus: `{report['corpus']['manifest_digest']}` ({report['corpus']['logical_bytes']} bytes)",
        "- Wall time includes SSH setup, transfer/teardown, and an independent remote manifest.",
        "",
        "| Method | Transport | Streams | Compression | Median wall | MAD | Setup | Wire | Paired speedup |",
        "|---|---|---:|---|---:|---:|---:|---:|---:|",
    ]
    for method in report["methods"]:
        summary = method["summary"]
        speedup = f"{summary['paired_speedup']:.3f}x" if "paired_speedup" in summary else "-"
        config = method["config"]
        lines.append(
            f"| {method['name']} | {config['transport']} | {config['streams']} | {config['compression']} | "
            f"{summary['median_wall_seconds']:.6f}s | {summary['mad_wall_seconds']:.6f}s | "
            f"{summary['median_setup_seconds']:.6f}s | {summary['median_wire_bytes']} | {speedup} |"
        )
    lines.extend([
        "",
        "## Transport capability matrix",
        "",
        "| Backend | Status | Dialect | Features | Correctness | Degraded guarantees |",
        "|---|---|---|---|---|---|",
    ])
    for name, capability in report["transport_capabilities"].items():
        lines.append(
            f"| {name} | {capability['status']} | {capability['dialect']} | "
            f"{', '.join(capability['features'])} | "
            f"{capability['correctness']} | {', '.join(capability['degraded_guarantees']) or '-'} |"
        )
    lines.extend([
        "",
        "## Repetitions",
        "",
        "| Method | Rep | Order | Wall | CPU | RSS | Wire | Setup | Verify | Oracle |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---|",
    ])
    for method in report["methods"]:
        for sample in method["samples"]:
            phases = sample["phases_seconds"]
            lines.append(
                f"| {method['name']} | {sample['repetition']} | {sample['method_order']} | "
                f"{sample['wall_seconds']:.6f}s | {sample['cpu_seconds']:.6f}s | "
                f"{sample['peak_rss_bytes']} | {sample['wire_bytes']} | "
                f"{phases['ssh_setup']:.6f}s | {phases['independent_verification']:.6f}s | "
                f"{'pass' if sample['oracle']['passed'] else 'FAIL'} |"
            )
    return "\n".join(lines) + "\n"


def atomic_write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temporary.write_text(text)
    temporary.replace(path)


def main() -> None:
    args = parser().parse_args()
    if args.repetitions < 5:
        raise SystemExit("at least five repetitions are required")
    expected = json.loads(args.manifest.read_text())
    run_capture(ssh(args.host, args.remote_binary, "prepare", "--root", args.destination_base))
    methods = args.methods.split(",")
    if len(set(methods)) != len(methods):
        raise SystemExit("methods must be unique")
    results = [{"name": name, "config": method_config(name, args), "samples": []} for name in methods]
    by_name = {result["name"]: result for result in results}
    for repetition in range(args.repetitions):
        for method_order, method in enumerate(schedule(methods, repetition)):
            safe_method = method.replace("-", "_")
            target = f"{args.destination_base}/rep_{repetition}_{method_order}_{safe_method}"
            print(f"rep {repetition + 1}/{args.repetitions} order {method_order}: {method}", flush=True)
            if method.startswith("rsync"):
                sample = rsync_method(args, method, target, expected)
            else:
                sample = native_method(args, method, target, expected)
            sample.update({"repetition": repetition, "method_order": method_order, "cache_state": "first_pass" if repetition == 0 and method_order == 0 else "warm"})
            if not sample["oracle"]["passed"]:
                raise RuntimeError(f"independent oracle failed for {method} repetition {repetition}")
            by_name[method]["samples"].append(sample)
    summarize(results)
    remote_version = run_capture(ssh(args.host, "rsync", "--version")).stdout.splitlines()[0]
    protocol_match = re.search(r"protocol version (\d+)", remote_version)
    report = {
        "schema": SCHEMA,
        "generated_unix_nanos": time.time_ns(),
        "build": {
            "source_revision": subprocess.run(
                ["git", "rev-parse", "HEAD"],
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
            ).stdout.strip() or "unknown-no-vcs-metadata",
            "build_id": "xsync-remote-spike-0.1.0-release-xsync-story-0.5-spike-v1",
            "profile": "release",
        },
        "environment": {
            "local_host": os.uname().nodename,
            "local_os_kernel": " ".join(os.uname()),
            "local_hardware": run_capture(
                ["sysctl", "-n", "machdep.cpu.brand_string"]
            ).stdout.strip(),
            "remote_host": args.host,
            "remote_os_kernel": run_capture(
                ssh(args.host, "uname", "-srmo")
            ).stdout.strip(),
            "remote_hardware": run_capture(
                ssh(args.host, "sh", "-lc", "lscpu | sed -n 's/^Model name:[[:space:]]*//p' | head -1")
            ).stdout.strip(),
            "destination_filesystem": args.filesystem,
            "profile": args.profile,
            "route": f"ssh {os.uname().nodename} -> {args.host}",
            "receiver_prefix": args.receiver_prefix or "none",
        },
        "corpus": {
            "schema": expected["schema"],
            "manifest_digest": expected["manifest_digest"],
            "item_count": expected["item_count"],
            "logical_bytes": expected["logical_bytes"],
        },
        "tools": {
            "local_rsync": tool_version(["rsync", "--version"]),
            "remote_rsync": remote_version,
            "ssh": subprocess.run(["ssh", "-V"], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT).stdout.strip(),
            "native_spike": "xsync.remote-spike.send.v1",
        },
        "transport_capabilities": {
            "native_xsync": {
                "status": "benchmark_spike_only",
                "dialect": "xsync-story-0.5-spike-v1",
                "features": ["bounded frames", "1/2/4/8 persistent data sessions", "BLAKE3 before atomic publication", "adaptive zstd"],
                "correctness": "independent manifest required for every sample",
                "degraded_guarantees": ["flat regular-file corpus only", "no durable resume", "not production protocol v1"],
            },
            "rsync_protocol_fallback": {
                "status": "not_implemented_story_4.5",
                "dialect": f"reference receiver protocol {protocol_match.group(1) if protocol_match else 'unknown'}",
                "features": ["reference rsync -a/-az measured", "server setup marker measured"],
                "correctness": "reference client samples independently manifested; native codec unavailable",
                "degraded_guarantees": ["no xsync BLAKE3 framing", "no xsync checkpoint resume", "single stream", "native fallback setup time unavailable until Story 4.5"],
            },
        },
        "methods": results,
    }
    atomic_write(args.json, json.dumps(report, indent=2) + "\n")
    atomic_write(args.markdown, markdown(report))
    print(f"wrote {args.json} and {args.markdown}")


if __name__ == "__main__":
    main()
