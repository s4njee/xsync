#!/usr/bin/env python3
"""Story 8.1 release benchmark matrix.

Runs paired tool comparisons (xsync against a reference rsync) for a corpus
class, workload, and route; captures wall/CPU/peak-RSS per invocation; verifies
every produced destination with the independent xsync-bench manifest oracle; and
emits an `xsync.bench.input.v1` document per cell that is rendered through
`xsync-bench report` so the Epic 0 median/MAD/paired-ratio policy and
`xsync-bench gate` apply unchanged.

Unlike the earlier release-matrix.py smoke runner, every cell here carries a
same-run baseline and a rotated method order, which is what makes the result
gate-able rather than merely correct.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shlex
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
INPUT_SCHEMA = "xsync.bench.input.v1"

# ru_maxrss is bytes on macOS and kilobytes on Linux.
RSS_SCALE = 1 if sys.platform == "darwin" else 1024


class CellFailure(RuntimeError):
    """A cell could not produce a valid gate-able result."""


class DriftedCellFailure(CellFailure):
    """The source changed relative to its pinned manifest."""


# --------------------------------------------------------------------------
# process measurement
# --------------------------------------------------------------------------

def run_measured(command: list[str], cwd: Path | None = None) -> dict:
    """Run a command, returning wall/CPU/peak-RSS plus captured output.

    os.wait4 is used instead of Popen.wait so the kernel's rusage for this
    exact child is available; Popen must not reap the process first.
    """
    out_path = Path(f"/tmp/xsync-bench-stdout-{os.getpid()}")
    err_path = Path(f"/tmp/xsync-bench-stderr-{os.getpid()}")
    with out_path.open("wb") as out, err_path.open("wb") as err:
        started = time.monotonic()
        process = subprocess.Popen(command, stdout=out, stderr=err, cwd=cwd)
        _, status, usage = os.wait4(process.pid, 0)
        wall = time.monotonic() - started
        process.returncode = os.waitstatus_to_exitcode(status)
    return {
        "returncode": process.returncode,
        "wall_seconds": wall,
        "cpu_seconds": usage.ru_utime + usage.ru_stime,
        "peak_rss_bytes": int(usage.ru_maxrss) * RSS_SCALE,
        "stdout": out_path.read_text(encoding="utf-8", errors="replace"),
        "stderr": err_path.read_text(encoding="utf-8", errors="replace"),
    }


def done_event(stdout: str) -> dict:
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if value.get("type") == "done" or value.get("event") == "finished":
            return value
    return {}


def phase_timings(stdout: str) -> dict[str, float]:
    """Extract measured phase durations from timestamped progress-json events."""
    boundaries: dict[str, dict[str, int]] = {}
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("event") != "phase" or "timestamp_unix_nanos" not in event:
            continue
        name = event.get("name")
        if name not in {"scan", "plan", "transfer", "metadata"}:
            continue
        state = "start" if event.get("started") else "end"
        boundaries.setdefault(name, {})[state] = int(event["timestamp_unix_nanos"])
    return {
        name: (value["end"] - value["start"]) / 1_000_000_000
        for name, value in boundaries.items()
        if "start" in value and "end" in value and value["end"] >= value["start"]
    }


# --------------------------------------------------------------------------
# identity
# --------------------------------------------------------------------------

def git_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=REPO, text=True, capture_output=True
    )
    revision = result.stdout.strip() or "unknown"
    dirty = subprocess.run(
        ["git", "status", "--porcelain"], cwd=REPO, text=True, capture_output=True
    )
    return revision + ("-dirty" if dirty.stdout.strip() else "")


def build_id(binary: Path) -> str:
    digest = subprocess.run(
        ["shasum", "-a", "256", str(binary)], text=True, capture_output=True
    )
    return digest.stdout.split()[0][:32] if digest.stdout else "unknown"


def tool_version(command: list[str], first_line: bool = True) -> str:
    result = subprocess.run(command, text=True, capture_output=True)
    text = (result.stdout or result.stderr).strip()
    return text.splitlines()[0] if first_line and text else text or "unknown"


def hardware_description() -> str:
    if sys.platform == "darwin":
        cpu = tool_version(["sysctl", "-n", "machdep.cpu.brand_string"])
        cores = tool_version(["sysctl", "-n", "hw.ncpu"])
        memory = tool_version(["sysctl", "-n", "hw.memsize"])
        gib = int(memory) / (1024 ** 3) if memory.isdigit() else 0
        return f"{cpu}, {cores} logical cores, {gib:.0f} GiB RAM"
    return f"{platform.processor() or platform.machine()}, {os.cpu_count()} logical cores"


def filesystem_of(path: Path) -> str:
    result = subprocess.run(
        ["df", "-P", "-T" if sys.platform != "darwin" else "-k", str(path)],
        text=True, capture_output=True,
    )
    lines = result.stdout.splitlines()
    device = lines[1].split()[0] if len(lines) > 1 else "unknown"
    if sys.platform == "darwin":
        info = subprocess.run(
            ["diskutil", "info", device], text=True, capture_output=True
        ).stdout
        for line in info.splitlines():
            if "Type (Bundle)" in line:
                return f"{line.split(':')[-1].strip()} ({device})"
    return device


# --------------------------------------------------------------------------
# corpora
# --------------------------------------------------------------------------

@dataclass(frozen=True)
class RealCorpus:
    """A source-only corpus registered by the tuning plan."""

    name: str
    sources: tuple[Path, ...]
    root: Path
    pinned_digest: str | None = None
    workloads: tuple[str, ...] = (
        "initial-copy", "no-op-second-sync", "content-churn", "delete"
    )
    expected_file_count: int | None = None


def real_corpora() -> dict[str, RealCorpus]:
    root = Path(os.environ.get("XSYNC_CORPORA_DIR", REPO / "corpora")).expanduser()
    congress_root = root / "congress"
    congress = congress_root / "data" if (congress_root / "data").is_dir() else congress_root
    digests = {
        key.removeprefix("XSYNC_CORPUS_DIGEST_").lower().replace("_", "-"): value.strip()
        for key, value in os.environ.items()
        if key.startswith("XSYNC_CORPUS_DIGEST_") and value.strip()
    }
    # This is the verified digest recorded by the earlier congress-10k smoke run.
    digests.setdefault("congress-10k", "f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6")
    # Captured read-only on 2026-08-24 from csearchv2 congress/data/118.
    digests.setdefault("congress-100k", "2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794")
    # Captured from the two-subtree staged congress-1k source.
    digests.setdefault("congress-1k", "4bd58b6178805410354b76c506b881200b2ebaa28984e343a35d12ac9472c496")
    # Read-only captures on 2026-08-24.
    digests.setdefault("congress-1m", "0332417e2c92df6f27209ae0a84318c2eff1c7a4227dec1bc78dbfbcc592e7a5")
    digests.setdefault("manga", "78d09dfb56ce56a454c3c654c5c228899d2a5d2ab7167060bf6c186a927417ef")
    digests.setdefault("cb7", "239102af9740dca93110c3e2327831293214e05fb2bc432218dd00bca82fe14c")
    return {
        "congress-1k": RealCorpus(
            "congress-1k",
            (congress / "100/bills/hconres", congress / "100/bills/hjres"),
            congress_root,
            digests.get("congress-1k"),
            expected_file_count=1_076,
        ),
        "congress-10k": RealCorpus("congress-10k", (congress / "100",), congress_root, digests.get("congress-10k"), expected_file_count=11_280),
        "congress-100k": RealCorpus("congress-100k", (congress / "118",), congress_root, digests.get("congress-100k"), expected_file_count=109_615),
        "congress-1m": RealCorpus("congress-1m", (congress,), congress_root, digests.get("congress-1m"), expected_file_count=1_318_771),
        "manga": RealCorpus("manga", (root / "Manga",), root / "Manga", digests.get("manga"), expected_file_count=117),
        "cb7": RealCorpus("cb7", (root / "cb7",), root / "cb7", digests.get("cb7"), expected_file_count=204_577),
        "docker-raw": RealCorpus(
            "docker-raw", (root / "docker-raw/Docker.raw",), root / "docker-raw", digests.get("docker-raw"),
            expected_file_count=1,
        ),
    }


def validate_destination(destination: Path, corpora: dict[str, RealCorpus]) -> None:
    """Reject destinations inside a source corpus before any filesystem work."""
    destination = destination.expanduser().resolve()
    for corpus in corpora.values():
        root = corpus.root.expanduser().resolve()
        if destination == root or root in destination.parents:
            raise CellFailure(
                f"destination '{destination}' is inside real corpus '{corpus.name}' "
                "which is source-only"
            )


def resolve_real_corpus(name: str, corpora: dict[str, RealCorpus]) -> RealCorpus:
    try:
        corpus = corpora[name]
    except KeyError as error:
        choices = ", ".join(corpora)
        raise CellFailure(f"unknown real corpus '{name}'; choose one of: {choices}") from error
    missing = next((path for path in corpus.sources if not path.exists()), None)
    if missing:
        raise CellFailure(
            f"real corpus '{name}' is missing at '{missing}'; "
            "set XSYNC_CORPORA_DIR to its parent directory"
        )
    if not corpus.pinned_digest:
        raise CellFailure(
            f"real corpus '{name}' has no pinned manifest digest; set "
            f"XSYNC_CORPUS_DIGEST_{name.upper().replace('-', '_')} after an approved capture"
        )
    if len(corpus.pinned_digest) != 64 or any(c not in "0123456789abcdef" for c in corpus.pinned_digest):
        raise CellFailure(f"real corpus '{name}' has an invalid pinned manifest digest")
    return corpus


def assert_docker_stopped(corpus: RealCorpus) -> None:
    if corpus.name != "docker-raw":
        return
    desktop = subprocess.run(["pgrep", "-x", "Docker Desktop"], capture_output=True)
    backend = subprocess.run(["pgrep", "-f", "com.docker.backend"], capture_output=True)
    if desktop.returncode == 0 or backend.returncode == 0:
        raise CellFailure(
            "docker-raw requires Docker to be stopped; stop Docker Desktop and its backend "
            "before measuring the live Docker.raw source"
        )


def evict_cache() -> str:
    """Evict the local page cache and return the action recorded in reports."""
    if sys.platform == "darwin":
        command = shutil.which("purge")
        if not command:
            raise CellFailure("cold-cache mode requires macOS 'purge', but it is unavailable")
        result = subprocess.run([command], text=True, capture_output=True)
        if result.returncode:
            raise CellFailure(f"macOS purge failed: {result.stderr.strip()[:500]}")
        return "purge (macOS)"
    if sys.platform.startswith("linux"):
        result = subprocess.run(
            ["sudo", "-n", "sh", "-c", "sync; echo 3 > /proc/sys/vm/drop_caches"],
            text=True, capture_output=True,
        )
        if result.returncode:
            raise CellFailure(
                "cold-cache mode could not drop Linux caches without passwordless sudo: "
                f"{result.stderr.strip()[:400]}"
            )
        return "sync + drop_caches (Linux)"
    raise CellFailure(f"cold-cache mode is unsupported on {sys.platform}")


class MacNetworkShaper:
    """Opt-in macOS PF/dummynet shaper for one SSH receiver."""

    def __init__(self, host: str, bandwidth_mbit: int, latency_ms: int):
        if sys.platform != "darwin":
            raise CellFailure("bandwidth shaping is currently implemented only on macOS")
        hostname = host.rsplit("@", 1)[-1]
        try:
            address = socket.gethostbyname(hostname)
        except OSError as error:
            raise CellFailure(f"could not resolve shaping target '{hostname}': {error}") from error
        route = subprocess.run(
            ["route", "-n", "get", address], text=True, capture_output=True
        )
        interface = next(
            (line.split(":", 1)[1].strip() for line in route.stdout.splitlines()
             if line.strip().startswith("interface:")),
            None,
        )
        if route.returncode or not interface:
            raise CellFailure(f"could not determine interface for shaping target {address}")
        self.address = address
        self.interface = interface
        self.pipe = 30_000 + (os.getpid() % 10_000)
        self.anchor = f"xsync-bench/{os.getpid()}"
        self.pf_token: str | None = None
        self.bandwidth_mbit = bandwidth_mbit
        self.latency_ms = latency_ms

    @property
    def description(self) -> str:
        return f"dummynet {self.bandwidth_mbit} Mbit/s, {self.latency_ms} ms latency to {self.address}"

    def rules(self) -> str:
        return (
            f"dummynet out on {self.interface} proto tcp to {self.address} pipe {self.pipe}\n"
            f"dummynet in on {self.interface} proto tcp from {self.address} pipe {self.pipe}\n"
        )

    def start(self) -> None:
        result = subprocess.run(
            ["dnctl", "pipe", str(self.pipe), "config",
             "bw", f"{self.bandwidth_mbit}Mbit/s", "delay", f"{self.latency_ms}ms"],
            text=True, capture_output=True,
        )
        if result.returncode:
            raise CellFailure(f"dnctl pipe setup failed: {result.stderr.strip()[:500]}")
        enabled = subprocess.run(["pfctl", "-E"], text=True, capture_output=True)
        if enabled.returncode:
            self.stop()
            raise CellFailure(f"pfctl enable failed: {enabled.stderr.strip()[:500]}")
        self.pf_token = next(
            (line.split(":", 1)[1].strip() for line in enabled.stdout.splitlines()
             if line.lower().startswith("token")),
            None,
        )
        loaded = subprocess.run(
            ["pfctl", "-a", self.anchor, "-f", "-"],
            input=self.rules(), text=True, capture_output=True,
        )
        if loaded.returncode:
            self.stop()
            raise CellFailure(f"pfctl rule setup failed: {loaded.stderr.strip()[:500]}")

    def stop(self) -> None:
        subprocess.run(["pfctl", "-a", self.anchor, "-F", "all"],
                       text=True, capture_output=True)
        subprocess.run(["dnctl", "pipe", str(self.pipe), "delete"],
                       text=True, capture_output=True)
        if self.pf_token:
            subprocess.run(["pfctl", "-X", self.pf_token],
                           text=True, capture_output=True)

def make_corpus(bench: Path, base: Path, klass: str, tier: str,
                workload: str, seed: int) -> Path:
    base.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        [str(bench), "corpus", "--base", str(base), "--class", klass,
         "--tier", tier, "--workload", workload, "--seed", str(seed)],
        text=True, capture_output=True,
    )
    if result.returncode:
        raise CellFailure(f"corpus generation failed: {result.stderr.strip()}")
    return Path(result.stdout.strip().splitlines()[-1])


def make_real_manifest(bench: Path, corpus: RealCorpus, output: Path) -> dict:
    result = subprocess.run(
        [str(bench), "manifest", str(corpus.sources[0]), "--out", str(output)],
        text=True,
    )
    if result.returncode:
        raise CellFailure("manifest generation failed")
    return json.loads(output.read_text())


def validate_real_manifest(corpus: RealCorpus, payload: dict) -> str:
    """Validate the pinned layout and digest before a real-corpus measurement."""
    actual_file_count = sum(
        entry.get("kind") == "file" for entry in payload.get("entries", [])
    )
    if (corpus.expected_file_count is not None
            and actual_file_count != corpus.expected_file_count):
        raise CellFailure(
            f"real corpus '{corpus.name}' layout mismatch: expected "
            f"{corpus.expected_file_count} regular files, observed {actual_file_count}"
        )
    observed_digest = payload.get("manifest_digest")
    if observed_digest != corpus.pinned_digest:
        raise DriftedCellFailure(
            f"status: drifted; corpus '{corpus.name}' expected digest "
            f"{corpus.pinned_digest}, observed {observed_digest}"
        )
    return observed_digest


def seed_destination(run_root: Path, destination: Path) -> float:
    """Reset the destination to the corpus's pinned initial state."""
    started = time.monotonic()
    if destination.exists():
        shutil.rmtree(destination)
    template = run_root / "destination"
    destination.parent.mkdir(parents=True, exist_ok=True)
    if template.exists() and any(template.iterdir()):
        # cp -Rp preserves mode and mtime and is far faster than copytree.
        result = subprocess.run(
            ["cp", "-Rp", str(template), str(destination)],
            text=True, capture_output=True,
        )
        if result.returncode:
            raise CellFailure(f"destination seeding failed: {result.stderr.strip()}")
    else:
        destination.mkdir(parents=True, exist_ok=True)
    if template.exists():
        shutil.copystat(template, destination, follow_symlinks=False)
    return time.monotonic() - started


def seed_real_destination(source: Path, destination: Path, workload: str,
                          seed: int) -> tuple[float, list[str]]:
    """Prepare a real-corpus destination without ever modifying the source."""
    started = time.monotonic()
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)
    selected: list[str] = []
    if workload == "initial-copy":
        return time.monotonic() - started, selected

    if workload not in {"no-op-second-sync", "content-churn", "delete"}:
        raise CellFailure(f"real corpus workload '{workload}' has no safe derivation rule")
    result = subprocess.run(
        ["rsync", "-a", f"{source}/", f"{destination}/"],
        text=True, capture_output=True,
    )
    if result.returncode:
        raise CellFailure(f"real destination preparation failed: {result.stderr.strip()[:500]}")
    if workload == "no-op-second-sync":
        return time.monotonic() - started, selected

    candidates = sorted(
        path for path in destination.rglob("*")
        if path.is_file()
        and not path.is_symlink()
        and (workload == "delete" or path.stat().st_size > 0)
    )
    target_count = max(1, (len(candidates) + 99) // 100) if candidates else 0
    ranked = sorted(
        candidates,
        key=lambda path: hashlib.blake2b(
            f"{seed}:".encode() + path.relative_to(destination).as_posix().encode(),
            digest_size=8,
        ).digest(),
    )
    for path in ranked[:target_count]:
        relative = path.relative_to(destination).as_posix()
        selected.append(relative)
        if workload == "delete":
            path.unlink()
        elif path.stat().st_size:
            with path.open("r+b") as handle:
                first = handle.read(1)
                handle.seek(0)
                handle.write(bytes([first[0] ^ 1]))
    return time.monotonic() - started, selected


def verify(bench: Path, destination: Path, manifest: Path,
           sample_fraction: float | None = None, sample_seed: int = 0) -> tuple[dict, float]:
    started = time.monotonic()
    report_path = Path(f"/tmp/xsync-bench-verify-{os.getpid()}.json")
    command = [str(bench), "verify", str(destination), "--manifest", str(manifest),
               "--json", str(report_path)]
    if sample_fraction is not None:
        command.extend(["--sample", str(sample_fraction), "--sample-seed", str(sample_seed)])
    result = subprocess.run(
        command,
        text=True,
    )
    elapsed = time.monotonic() - started
    if report_path.exists():
        payload = json.loads(report_path.read_text())
    else:
        payload = {
            "passed": False, "expected_manifest_digest": "", "actual_manifest_digest": "",
            "item_count": 0, "logical_bytes": 0, "mismatch_count": 1, "mismatches": [],
        }
    payload["passed"] = payload.get("passed", False) and result.returncode == 0
    return payload, elapsed


# --------------------------------------------------------------------------
# remote (ssh) destinations
# --------------------------------------------------------------------------

def remote_run(host: str, command: str, check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", host, command], text=True, capture_output=True
    )
    if check and result.returncode:
        raise CellFailure(f"remote command failed on {host}: {result.stderr.strip()[:500]}")
    return result


def seed_destination_ssh(run_root: Path, host: str, remote_dest: str, workload: str) -> float:
    """Reset a remote destination to the corpus's pinned initial state."""
    started = time.monotonic()
    remote_run(host, f"rm -rf {shlex.quote(remote_dest)} && mkdir -p {shlex.quote(remote_dest)}")
    template = run_root / "destination"
    if workload != "initial-copy" and template.exists() and any(template.iterdir()):
        result = subprocess.run(
            ["rsync", "-a", "--delete", f"{template}/", f"{host}:{remote_dest}/"],
            text=True, capture_output=True,
        )
        if result.returncode:
            raise CellFailure(f"remote seeding failed: {result.stderr.strip()[:500]}")
    return time.monotonic() - started


def verify_ssh(host: str, remote_dest: str, remote_manifest: str,
               remote_bench: str, sample_fraction: float | None = None,
               sample_seed: int = 0) -> tuple[dict, float]:
    """Verify a remote destination with the remote independent oracle."""
    started = time.monotonic()
    probe = "/tmp/xsync-bench-verify.json"
    sampling = ""
    if sample_fraction is not None:
        sampling = f" --sample {sample_fraction} --sample-seed {sample_seed}"
    command = (
        f"{shlex.quote(remote_bench)} verify {shlex.quote(remote_dest)} "
        f"--manifest {shlex.quote(remote_manifest)} --json {shlex.quote(probe)}{sampling} "
        f">/dev/null 2>&1; cat {shlex.quote(probe)}"
    )
    result = remote_run(host, command, check=False)
    elapsed = time.monotonic() - started
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError:
        payload = {
            "passed": False, "expected_manifest_digest": "", "actual_manifest_digest": "",
            "item_count": 0, "logical_bytes": 0, "mismatch_count": 1, "mismatches": [],
        }
    return payload, elapsed


# --------------------------------------------------------------------------
# methods
# --------------------------------------------------------------------------

def build_methods(args: argparse.Namespace, route: str, workload: str,
                  source: Path, destination: Path, wrapper: Path) -> dict:
    """Return {method_name: (command, description)} for a route/workload."""
    checksum = workload == "content-churn"
    deleting = workload in {"delete", "interrupted-resume"}

    xsync = [str(args.xsync), "--progress-json"]
    rsync = [args.rsync, "-a"]
    if checksum:
        xsync.append("--checksum")
        rsync.append("-c")
    if deleting:
        xsync.append("--delete")
        rsync.append("--delete")

    methods: dict[str, list[str]] = {}
    if route == "ssh":
        # destination is already a "host:/path" argument for this route.
        methods["rsync-a"] = rsync + [f"{source}/", f"{destination}/"]
        methods["xsync"] = xsync + ["--transport=xsync", "-e", str(wrapper),
                                    f"{source}/", f"{destination}/"]
        methods["xsync-rsync-transport"] = xsync + ["--transport", "rsync",
                                                    f"{source}/", f"{destination}/"]
    elif route in {"same-volume", "cross-volume"}:
        methods["rsync-a"] = rsync + [f"{source}/", f"{destination}/"]
        methods["xsync"] = xsync + [f"{source}/", f"{destination}/"]
    else:
        host = platform.node()
        methods["rsync-a"] = rsync + ["-e", str(wrapper),
                                      f"{source}/", f"{host}:{destination}/"]
        methods["rsync-az"] = rsync + ["-z", "-e", str(wrapper),
                                       f"{source}/", f"{host}:{destination}/"]
        methods["xsync"] = xsync + ["--transport=xsync", "-e", str(wrapper),
                                    f"{source}/", f"{host}:{destination}/"]
        methods["xsync-raw"] = xsync + ["--transport=xsync", "--no-compress",
                                        "-e", str(wrapper),
                                        f"{source}/", f"{host}:{destination}/"]
    return methods


def write_wrappers(directory: Path, xsync: Path) -> tuple[Path, Path]:
    directory.mkdir(parents=True, exist_ok=True)
    xsync_wrapper = directory / "xsync-rsh.sh"
    xsync_wrapper.write_text(
        "#!/bin/sh\nshift\neval \"set -- $1\"\n"
        f"exec {shlex.quote(str(xsync))} \"$2\" \"$3\"\n",
        encoding="utf-8",
    )
    xsync_wrapper.chmod(0o755)
    rsync_wrapper = directory / "rsync-rsh.sh"
    rsync_wrapper.write_text("#!/bin/sh\nshift\nexec \"$@\"\n", encoding="utf-8")
    rsync_wrapper.chmod(0o755)
    return xsync_wrapper, rsync_wrapper


def write_ssh_wrapper(directory: Path, remote_bin_dir: str) -> Path:
    """An rsh wrapper that prepends the remote release directory to PATH.

    xsync invokes `rsh <host> "'xsync' '--server' '<path>'"`, so the remote
    `xsync` must resolve on a non-interactive PATH. This avoids modifying the
    remote account's shell configuration.
    """
    directory.mkdir(parents=True, exist_ok=True)
    wrapper = directory / "ssh-rsh.sh"
    wrapper.write_text(
        "#!/bin/sh\n"
        "host=$1\n"
        "shift\n"
        f"exec ssh -o BatchMode=yes \"$host\" \"export PATH={remote_bin_dir}:\\$PATH; $*\"\n",
        encoding="utf-8",
    )
    wrapper.chmod(0o755)
    return wrapper


# --------------------------------------------------------------------------
# one cell
# --------------------------------------------------------------------------

def run_cell(args: argparse.Namespace, klass: str, workload: str, route: str,
             destination_root: Path, output_root: Path,
             real_corpus: RealCorpus | None = None) -> dict:
    label = f"{klass}-{workload}-{route}"
    print(f"  [cell] {label} ...", flush=True)

    if real_corpus:
        run_root = args.scratch / f"real-{klass}"
        run_root.mkdir(parents=True, exist_ok=True)
        manifest = output_root / f"manifest-{klass}.json"
        if len(real_corpus.sources) == 1:
            source = real_corpus.sources[0]
        else:
            source = run_root / "source"
            if not source.exists():
                source.mkdir()
                for path in real_corpus.sources:
                    shutil.copytree(path, source / path.name, symlinks=True)
        staged_corpus = RealCorpus(
            real_corpus.name,
            (source,),
            real_corpus.root,
            real_corpus.pinned_digest,
            real_corpus.workloads,
            real_corpus.expected_file_count,
        )
        manifest_payload = make_real_manifest(args.bench, staged_corpus, manifest)
        validate_real_manifest(real_corpus, manifest_payload)
        scenario = {
            "corpus_schema": "xsync.manifest.v1",
            "expected": {
                "file": str(manifest),
                "digest": manifest_payload["manifest_digest"],
                "allocated_bytes": manifest_payload.get("allocated_bytes", 0),
            },
        }
    else:
        run_root = make_corpus(args.bench, args.scratch, klass, args.tier, workload, args.seed)
        scenario = json.loads((run_root / "scenario.json").read_text())
        source = run_root / scenario["source"]
        manifest = run_root / scenario["expected"]["file"]

    wrapper_dir = destination_root / "wrappers"
    xsync_wrapper, rsync_wrapper = write_wrappers(wrapper_dir, args.xsync.resolve())

    remote_manifest = None
    if real_corpus and route == "ssh" and workload != "initial-copy":
        raise CellFailure(
            "real-corpus mutation workloads are currently local-route only; "
            "remote destination mutation is not enabled"
        )
    if route == "ssh":
        remote_dest = f"{args.ssh_destination}/dest-{label}"
        destination = f"{args.ssh_host}:{remote_dest}"
        xsync_wrapper = write_ssh_wrapper(wrapper_dir, args.remote_bin_dir)
        remote_manifest = f"{args.ssh_destination}/expected-{label}.json"
        remote_run(args.ssh_host, f"mkdir -p {shlex.quote(args.ssh_destination)}")
        copied = subprocess.run(
            ["scp", "-q", str(manifest), f"{args.ssh_host}:{remote_manifest}"],
            text=True, capture_output=True,
        )
        if copied.returncode:
            raise CellFailure(f"could not copy manifest to remote: {copied.stderr.strip()}")
    else:
        remote_dest = None
        destination = destination_root / f"dest-{label}"

    probe = build_methods(args, route, workload, source, destination, xsync_wrapper)
    names = list(probe.keys())

    schedule_path = output_root / f"schedule-{label}.json"
    schedule_result = subprocess.run(
        [str(args.bench), "schedule", "--methods", *names,
         "--repetitions", str(args.repetitions), "--out", str(schedule_path)],
        text=True, capture_output=True,
    )
    if schedule_result.returncode:
        raise CellFailure(f"schedule failed: {schedule_result.stderr.strip()}")
    schedule = json.loads(schedule_path.read_text())

    samples: dict[str, list[dict]] = {name: [] for name in names}
    for repetition, order in enumerate(schedule):
        for position, name in enumerate(order):
            source_digest = scenario["expected"]["digest"]
            if real_corpus:
                drift_probe = output_root / f"manifest-{klass}-rep{repetition}-method{position}.json"
                observed = validate_real_manifest(
                    real_corpus, make_real_manifest(args.bench, staged_corpus, drift_probe)
                )
                source_digest = observed
            mutation_selection: list[str] = []
            cache_eviction_method = None
            if args.cache_state == "cold" and repetition > 0:
                if route == "ssh":
                    raise CellFailure("cold-cache mode is local-host only; remote cache state is unknown")
                cache_eviction_method = evict_cache()
            if route == "ssh":
                wrapper = xsync_wrapper
            else:
                wrapper = rsync_wrapper if name.startswith("rsync") else xsync_wrapper
            commands = build_methods(args, route, workload, source, destination, wrapper)
            if route == "ssh":
                seed_seconds = seed_destination_ssh(
                    run_root, args.ssh_host, remote_dest, workload
                )
            else:
                if real_corpus:
                    seed_seconds, mutation_selection = seed_real_destination(
                        source, destination, workload, args.seed
                    )
                else:
                    seed_seconds = seed_destination(run_root, destination)
            print(
                f"    [run] repetition {repetition + 1}/{len(schedule)} "
                f"{name} ({position + 1}/{len(order)}) ...",
                flush=True,
            )
            measured = run_measured(commands[name])
            print(
                f"    [done] {name} in {measured['wall_seconds']:.2f}s",
                flush=True,
            )
            if measured["returncode"]:
                raise CellFailure(
                    f"{name} exited {measured['returncode']}: {measured['stderr'][-1500:]}"
                )
            if route == "ssh":
                verification_sample = None if repetition == 0 else args.verify_sample
                oracle, verify_seconds = verify_ssh(
                    args.ssh_host, remote_dest, remote_manifest, args.remote_bench,
                    verification_sample, args.verify_seed,
                )
            else:
                verification_sample = None if repetition == 0 else args.verify_sample
                oracle, verify_seconds = verify(
                    args.bench, destination, manifest,
                    verification_sample, args.verify_seed,
                )
            if not oracle["passed"]:
                raise CellFailure(
                    f"{name} correctness oracle failed "
                    f"({oracle.get('mismatch_count')} mismatches)"
                )
            event = done_event(measured["stdout"])
            measured_phases = phase_timings(measured["stdout"])
            if not measured_phases:
                measured_phases = {"transfer": measured["wall_seconds"]}
            samples[name].append({
                "repetition": repetition,
                "method_order": position,
                "wall_seconds": measured["wall_seconds"],
                "cpu_seconds": measured["cpu_seconds"],
                "peak_rss_bytes": measured["peak_rss_bytes"],
                "item_count": oracle["item_count"],
                "logical_bytes": oracle["logical_bytes"],
                "source_allocated_bytes": scenario["expected"].get("allocated_bytes", 0),
                "destination_allocated_bytes": oracle.get("allocated_bytes", 0),
                "wire_bytes": int(event.get("wire_bytes", 0)),
                "phases_seconds": {
                    **measured_phases,
                },
                "seed_destination_seconds": seed_seconds,
                "verify_oracle_seconds": verify_seconds,
                "cache_state": "first_pass" if repetition == 0 else (
                    "cold_evicted" if cache_eviction_method else "warm"
                ),
                "cache_eviction_method": cache_eviction_method,
                "oracle": oracle,
                "source_manifest_digest": source_digest,
                "mutation_selection": mutation_selection,
            })

    if route == "ssh":
        remote_run(args.ssh_host, f"rm -rf {shlex.quote(remote_dest)}", check=False)
    elif destination.exists():
        shutil.rmtree(destination)

    results = []
    for name in names:
        results.append({
            "name": name,
            "baseline": None if name == "rsync-a" else "rsync-a",
            "samples": samples[name],
        })

    document = {
        "schema": INPUT_SCHEMA,
        "build": {
            "source_revision": git_revision(),
            "build_id": build_id(args.xsync),
            "profile": "release",
        },
        "environment": {
            "hardware": hardware_description(),
            "os": platform.platform(),
            "kernel": platform.release(),
            "filesystem": args.ssh_filesystem if route == "ssh"
            else filesystem_of(destination_root),
            "transport": {
                "ssh": f"ssh to {args.ssh_host}",
                "pipe": "pipe (child xsync --server over stdio)",
            }.get(route, "local"),
            "route": route,
            "shaping": getattr(args, "shaping_description", "none"),
        },
        "session": {
            "streams": 1,
            "compression": "adaptive zstd" if route in {"pipe", "ssh"}
            else "none (local route)",
        },
        "corpus": {
            "schema": scenario["corpus_schema"],
            "manifest_digest": scenario["expected"]["digest"],
            "description": (
                f"real corpus={klass} pinned_digest={scenario['expected']['digest']}"
                if real_corpus else
                f"class={klass} tier={args.tier} workload={workload} seed={args.seed}"
            ),
        },
        "tools": [
            {"name": "xsync", "version": tool_version([str(args.xsync), "--version"]),
             "command": "xsync --progress-json SRC/ DEST/"},
            {"name": "rsync", "version": tool_version([args.rsync, "--version"]),
             "command": "rsync -a SRC/ DEST/"},
        ],
        "results": results,
    }

    input_path = output_root / f"input-{label}.json"
    input_path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    json_path = output_root / f"report-{label}.json"
    markdown_path = output_root / f"report-{label}.md"
    built = subprocess.run(
        [str(args.bench), "report", "--input", str(input_path),
         "--json", str(json_path), "--markdown", str(markdown_path)],
        text=True, capture_output=True,
    )
    if built.returncode:
        raise CellFailure(f"report rejected the samples: {built.stderr.strip()}")

    report = json.loads(json_path.read_text())
    try:
        report_reference = str(json_path.relative_to(REPO))
    except ValueError:
        report_reference = str(json_path)
    summary = {"cell": label, "class": klass, "workload": workload, "route": route,
               "status": "passed", "report": report_reference,
               "methods": {}}
    for result in report["results"]:
        summary["methods"][result["name"]] = {
            "median_wall_seconds": result["median_wall_seconds"],
            "mad_wall_seconds": result["mad_wall_seconds"],
            "median_cpu_seconds": result["median_cpu_seconds"],
            "peak_rss_bytes": result["peak_rss_bytes"],
            "median_wire_bytes": result["median_wire_bytes"],
            "paired_ratio_median": result.get("paired_ratio_median"),
            "paired_ratio_mad": result.get("paired_ratio_mad"),
        }
    return summary


# --------------------------------------------------------------------------
# driver
# --------------------------------------------------------------------------

DEFAULT_CELLS = [
    ("mixed", "initial-copy"),
    ("mixed", "no-op-second-sync"),
    ("mixed", "content-churn"),
    ("mixed", "metadata-only-churn"),
    ("mixed", "delete"),
    ("mixed", "type-replacement"),
    ("mixed", "interrupted-resume"),
    ("deep-small", "initial-copy"),
    ("compressible", "initial-copy"),
    ("incompressible", "initial-copy"),
    ("one-large-file", "initial-copy"),
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scratch", type=Path, default=Path("/tmp/xsync-bench-scratch"))
    parser.add_argument("--same-volume", type=Path, default=Path("/tmp/xsync-bench-dest"))
    parser.add_argument("--cross-volume", type=Path)
    parser.add_argument("--routes", default="same-volume,pipe")
    parser.add_argument("--tier", default="smoke")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument(
        "--verify-sample", type=float,
        help="sample fraction for verification after repetition 1; repetition 1 is always full",
    )
    parser.add_argument("--verify-seed", type=int, default=0,
                        help="seed for deterministic sampled verification")
    parser.add_argument("--cache-state", choices=("default", "cold"), default="default",
                        help="use real local cache eviction before repetitions after the first")
    parser.add_argument(
        "--bandwidth-mbit", type=int, choices=(50, 100, 1000),
        help="shape SSH traffic to 50, 100, or 1000 Mbit/s using macOS PF/dummynet",
    )
    parser.add_argument("--latency-ms", type=int, default=0,
                        help="added one-way latency for --bandwidth-mbit")
    parser.add_argument("--cells", help="comma-separated class:workload pairs")
    parser.add_argument(
        "--corpus", choices=tuple(real_corpora()),
        help="run one named real corpus; replaces --cells",
    )
    parser.add_argument(
        "--workload",
        choices=("initial-copy", "no-op-second-sync", "content-churn", "delete"),
        default="initial-copy",
        help="workload for --corpus (mutation states affect only the destination)",
    )
    parser.add_argument("--xsync", type=Path, default=REPO / "target/release/xsync")
    parser.add_argument("--bench", type=Path, default=REPO / "target/release/xsync-bench")
    parser.add_argument("--rsync", default="rsync")
    parser.add_argument("--ssh-host", help="user@host receiver for the ssh route")
    parser.add_argument("--ssh-destination", default="/tmp/xsync-release-bench",
                        help="remote directory that owns this run's destinations")
    parser.add_argument("--remote-bin-dir", default="",
                        help="remote directory holding the release xsync binary")
    parser.add_argument("--remote-bench", default="xsync-bench",
                        help="remote xsync-bench path used as the independent oracle")
    parser.add_argument("--ssh-filesystem", default="unknown",
                        help="filesystem backing the remote destination")
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.repetitions < 5:
        raise SystemExit("--repetitions must be at least 5 for a release gate")
    args.out.mkdir(parents=True, exist_ok=True)

    registry = real_corpora()
    selected_real = None
    if args.corpus:
        if args.cells:
            raise SystemExit("--corpus cannot be combined with --cells")
        selected_real = resolve_real_corpus(args.corpus, registry)
        assert_docker_stopped(selected_real)
        cells = [(args.corpus, args.workload)]
    elif args.cells:
        cells = [tuple(pair.split(":", 1)) for pair in args.cells.split(",")]
    else:
        cells = DEFAULT_CELLS

    if selected_real and args.tier != "smoke":
        # Real corpora have their own fixed tiers; --tier only sizes synthetic fixtures.
        print(f"note: ignoring synthetic tier '{args.tier}' for real corpus '{args.corpus}'",
              file=sys.stderr)

    routes = [route.strip() for route in args.routes.split(",") if route.strip()]
    if args.latency_ms < 0:
        raise SystemExit("--latency-ms must be non-negative")
    summaries: list[dict] = []
    for route in routes:
        if route == "ssh" and not args.ssh_host:
            summaries.append({"route": route, "status": "blocked",
                              "reason": "--ssh-host was not supplied"})
            continue
        if route == "cross-volume":
            root = args.cross_volume
            if root is None:
                summaries.append({"route": route, "status": "blocked",
                                  "reason": "--cross-volume was not supplied"})
                continue
        else:
            root = args.same_volume
        shaper = None
        args.shaping_description = "none"
        if args.bandwidth_mbit is not None:
            if route != "ssh":
                summaries.append({
                    "route": route, "status": "blocked",
                    "reason": "bandwidth shaping currently requires the ssh route",
                })
                continue
            if not args.ssh_host:
                summaries.append({
                    "route": route, "status": "blocked",
                    "reason": "bandwidth shaping requires --ssh-host",
                })
                continue
            try:
                shaper = MacNetworkShaper(
                    args.ssh_host, args.bandwidth_mbit, args.latency_ms
                )
                shaper.start()
                args.shaping_description = shaper.description
            except CellFailure as error:
                summaries.append({"route": route, "status": "blocked", "reason": str(error)})
                if shaper:
                    shaper.stop()
                continue
        validate_destination(root, registry)
        root.mkdir(parents=True, exist_ok=True)
        print(f"[route] {route} -> {root}", flush=True)
        try:
            for klass, workload in cells:
                try:
                    real = selected_real if selected_real and klass == selected_real.name else None
                    if real and workload not in real.workloads:
                        raise CellFailure(
                            f"real corpus '{real.name}' does not support workload '{workload}'"
                        )
                    summaries.append(run_cell(args, klass, workload, route, root, args.out, real))
                except CellFailure as error:
                    print(f"    FAILED: {error}", flush=True)
                    summaries.append({
                        "cell": f"{klass}-{workload}-{route}", "class": klass,
                        "workload": workload, "route": route,
                        "status": "drifted" if isinstance(error, DriftedCellFailure) else "failed",
                        "reason": str(error),
                    })
        finally:
            if shaper:
                shaper.stop()

    if "ssh" not in routes:
        summaries.append({
            "route": "ssh", "status": "blocked",
            "reason": "the ssh route was not selected; native xsync-over-SSH and "
                      "RsyncTransport rows require --routes ssh with --ssh-host",
        })

    matrix = {
        "schema": "xsync.release-bench.v1",
        "generated_unix_seconds": int(time.time()),
        "tier": args.tier,
        "seed": args.seed,
        "repetitions": args.repetitions,
        "routes": routes,
        "cells": summaries,
    }
    (args.out / "matrix.json").write_text(json.dumps(matrix, indent=2) + "\n", encoding="utf-8")
    write_matrix_markdown(args.out / "matrix.md", matrix)
    passed = sum(1 for row in summaries if row.get("status") == "passed")
    failed = sum(1 for row in summaries if row.get("status") == "failed")
    print(f"\n{passed} cells passed, {failed} failed -> {args.out / 'matrix.md'}")
    return 1 if failed else 0


NOISE_LIMIT = 0.15


def noise_ratio(values: dict) -> float:
    """MAD as a fraction of the median, the Epic 0 comparability signal."""
    median = values.get("median_wall_seconds") or 0.0
    if median <= 0:
        return 1.0
    return values.get("mad_wall_seconds", 0.0) / median


def write_matrix_markdown(path: Path, matrix: dict) -> None:
    lines = [
        "# xsync Story 8.1 release benchmark matrix",
        "",
        f"Tier `{matrix['tier']}`, seed `{matrix['seed']}`, "
        f"{matrix['repetitions']} repetitions per method, "
        "rotated method order, independent manifest oracle per run.",
        "",
        "`ratio` is the same-repetition paired speedup of xsync against `rsync -a` "
        "(above 1.0 means xsync is faster). Per the Epic 0 policy a row is "
        "comparable only when both it and its baseline hold MAD/median at or "
        "below 15%; rows marked `noisy` are reported but are **not** gate-able "
        "evidence.",
        "",
        "| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for row in matrix["cells"]:
        if row.get("status") != "passed":
            lines.append(
                f"| {row.get('cell', row.get('route'))} | {row.get('route','-')} | "
                f"*{row['status']}* | - | - | - | - | - | - | {row.get('reason','')[:120]} |"
            )
            continue
        baseline_noise = noise_ratio(row["methods"].get("rsync-a", {}))
        for name, values in row["methods"].items():
            ratio = values["paired_ratio_median"]
            ratio_text = f"{ratio:.3f}" if ratio else "baseline"
            noise = noise_ratio(values)
            worst = max(noise, baseline_noise)
            comparable = "yes" if worst <= NOISE_LIMIT else f"noisy ({worst:.0%})"
            if name == "rsync-a":
                comparable = "baseline" if noise <= NOISE_LIMIT else f"noisy ({noise:.0%})"
            lines.append(
                f"| {row['cell']} | {row['route']} | {name} | "
                f"{values['median_wall_seconds']:.4f} | {noise:.1%} | "
                f"{values['median_cpu_seconds']:.4f} | {values['peak_rss_bytes']:,} | "
                f"{values['median_wire_bytes']:,} | {ratio_text} | {comparable} |"
            )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
