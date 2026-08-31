# xsync

**A file synchronization tool that does the same job as `rsync`, written from scratch in Rust.**

You point it at a source and a destination, and it makes the destination look like the
source. The destination can be another directory on the same machine, or a directory on
another machine reachable over SSH. The command-line shape is deliberately familiar:

```bash
xs ~/Documents/notes/ freya:/srv/backup/notes/
```

The binary is called `xs`. The engine it drives is a Rust library called `xsync-core`,
and everything the CLI can do, an embedding application can do by calling that library
directly.

---

## Please read this first

**This is an instructive project, not production software.** It exists to explore how a
file synchroniser actually behaves once you measure it, and to be useful to anyone
building something similar. It is not a replacement for `rsync` and should not be trusted
with data you cannot afford to lose.

What that means concretely: the wire protocol is deliberately not frozen and has changed
several times, `Ctrl-C` has no signal handling and can leave staging files behind, and
`--delete` has a guard but no undo. None of that is oversight; it is a project that has
prioritised understanding over hardening.

The parts most likely to be worth your time are the ones that were *wrong first*. The
backlog and benchmark documents record the measurements that failed as carefully as the
ones that worked -- hypotheses that looked obvious and turned out to be false, numbers
that had to be retracted, and comparisons that were not measuring what they appeared to
be. If you take one thing from this repository, take the methodology: alternate your
arms, anchor on a control, verify exit status and landed file count, and be suspicious of
any benchmark where the two tools are not doing the same work.

If you want to sync files today, use `rsync`. If you want to know why it is fast, or to
build something in this space yourself, this may help.

---

## Table of contents

0. [Please read this first](#please-read-this-first)
1. [What this project actually is](#1-what-this-project-actually-is)
2. [Current status, stated honestly](#2-current-status-stated-honestly)
3. [Installing and building](#3-installing-and-building)
4. [Quick start](#4-quick-start)
5. [The complete command reference](#5-the-complete-command-reference)
6. [How it works, end to end](#6-how-it-works-end-to-end)
7. [Every feature, explained in plain English](#7-every-feature-explained-in-plain-english)
8. [The wire protocol](#8-the-wire-protocol)
9. [Protocol v2 and the browse surface](#9-protocol-v2-and-the-browse-surface)
10. [What xsync does not do](#10-what-xsync-does-not-do)
11. [Performance, with the actual numbers](#11-performance-with-the-actual-numbers)
12. [The benchmark harness](#12-the-benchmark-harness)
13. [Operating it: state on disk, cleanup, troubleshooting](#13-operating-it-state-on-disk-cleanup-troubleshooting)
14. [Roadmap to rsync quality](#14-roadmap-to-rsync-quality)
15. [Repository layout](#15-repository-layout)
16. [Document index](#16-document-index)

---

## 1. What this project actually is

rsync has been the default answer for "copy these files there, but only the parts that
changed" for thirty years. It is excellent, it is everywhere, and it was designed for
hardware that no longer exists — single-core machines on slow, expensive links, where the
overriding priority was to send as few bytes as possible even if that cost a lot of CPU
and a lot of round trips.

Modern hardware inverts several of those assumptions. Disks are fast, cores are plentiful,
LAN links are cheap, hashing is nearly free on modern CPUs, and some filesystems can
duplicate a whole directory tree without copying any bytes at all. xsync is an attempt to
build a sync engine that starts from *those* assumptions:

- **A bounded parallel pipeline** instead of a single-threaded loop, so scanning, hashing,
  reading and writing overlap.
- **BLAKE3** instead of MD5 for integrity, because it is dramatically faster and the CPU
  cost of verifying everything stops being a reason not to.
- **Filesystem-native cloning** (APFS `clonefile`, Linux reflinks) as a first-class fast
  path, because on a copy-on-write filesystem the fastest possible byte copy is no byte
  copy.
- **A modern framed protocol** with explicit, checked limits on every length, so a
  malicious or corrupt peer cannot make the receiver allocate unbounded memory.
- **Adaptive compression** decided by sampling the actual bytes, not by looking at the
  file extension.
- **Durable resume** for large files, so an interrupted 40 GB transfer does not start over.

The project is deliberately **evidence-driven**. An earlier version of the plan claimed
xsync would be "5–10x faster than rsync". That claim was retracted, in writing, because the
experiments behind it compared compressed archives against uncompressed rsync and often ran
a single repetition. The current rule, recorded in [`plan.md`](plan.md), is that **no
performance number is published unless it comes from the project's own benchmark harness,
paired against an rsync baseline measured in the same run, with at least five repetitions,
and an independent oracle confirming the destination is byte-correct.** Section 11 of this
README follows that rule, including where xsync currently loses.

---

## 2. Current status, stated honestly

**Version `0.1.0`. Native sync wire protocol v2 is current. Browse protocol v2 is exposed as a
library surface. This is pre-release software with no packaging and no signing.**

### The test suite

Run against the current tree (`c142c677`), the entire workspace is green:

| Test binary | Result |
|---|---|
| `xsync-core` unit tests | 140 passed, 2 ignored |
| `xsync-core` protocol v2 vectors | 4 passed |
| `xsync` CLI unit tests | 9 passed |
| `xsync` server integration tests | 23 passed |
| `xsync-bench` (harness) | 26 + 3 passed |
| `xsync-engine-bench` | 9 + 3 + 1 passed |
| **Total** | **218 passed, 0 failed** |

CI (`.github/workflows/ci.yml`) builds, tests and clippy-lints at `-D warnings` across
seven targets: macOS ARM64 and x86_64, Linux GNU and musl on both x86_64 and ARM64, and
Windows MSVC. It also enforces `cargo fmt` and a 1.88 MSRV. The whole workspace denies
`unsafe_code`.

> **Note on [`MVP.md`](MVP.md).** That document was written against an earlier, uncommitted
> working tree and reports eight failing integration tests, a broken Windows launcher, a
> `--delete` counter that under-reports on remote transfers, and an unresolved question
> about the remote `PATH` prefix. **All four of those were fixed in commit `c142c677`.**
> The suite is green, `xsync-server.cmd` now invokes `xs.exe` (and `build.rs` asserts it),
> the remote delete counter is incremented from a real acknowledgement, and the
> `PATH="$HOME/.local/bin:$PATH"` prefix is committed. MVP.md's operational guidance is
> still good; its defect list is stale.

### What works

| Capability | State |
|---|---|
| local → local | Works, with a directory-clone / reflink fast path |
| local → remote (push) | Works over SSH, native sync protocol v2 |
| remote → local (pull) | Works over SSH, native sync protocol v2 |
| remote → remote | **Not supported** — rejected at argument parsing |
| rsync-protocol fallback | Push and pull, GNU rsync protocol 32 only |
| Multi-stream striping | Implemented, 1–16 streams, **default 1** |
| Durable chunk resume | Implemented for large files |
| Browse/mutate session (v2) | Implemented as a **library API**; no CLI exposure |
| Windows | Works as a client; not usable as an SSH server (see §10) |
| Packaging, installers, signing | **None.** Build it yourself or stage a binary |

### What is missing before this is a product

No release artifacts (CI builds every target and uploads none), no code signing, no
package manager presence, no man page, no daemon or scheduler, and a placeholder
repository URL in `Cargo.toml`. Those are tracked in
[`DEPLOYMENT.md`](DEPLOYMENT.md) and summarized in §14.

Further correctness gaps are tracked in [`backlogv3.md`](backlogv3.md) — the remaining
ones are sparse files (see §10) and the metadata classes xsync silently drops.

**Fixed:** two source paths differing only in case, or in Unicode normalization, used to
become one file on a case- or normalization-insensitive destination such as APFS or NTFS.
A four-file Linux source pulled to macOS landed as two files with exit code 0. xsync now
probes the destination and refuses; see
[Destination path collisions](#destination-path-collisions).

---

## 3. Installing and building

### Requirements

Rust 1.88 or newer. Nothing else on the build host. On a *remote* host, xsync needs only
SSH and a filesystem — no Rust, no compiler, no root, no package manager.

### Build and install locally

```bash
cargo install --path crates/xsync
```

That puts `xs` in `~/.cargo/bin/xs`. Check it:

```bash
xs --version
```

### Cross-build for a Linux host from a Mac

```bash
brew install zig && cargo install cargo-zigbuild
```

```bash
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
```

### Stage a binary onto a remote host

`scripts/stage-linux.sh` cross-builds, uploads under a temporary name, verifies the
SHA-256 on the remote, and only then moves it into place. An interrupted upload leaves the
previous binary untouched, and re-running is safe.

```bash
scripts/stage-linux.sh freya "$HOME/.local/bin/xs" amd64
```

`$HOME` there expands on the *local* machine. If the remote username differs, use the
wrapper that resolves the remote home over SSH first:

```bash
scripts/deploy-mars.sh freya
```

Despite the name, `deploy-mars.sh [host] [remote-path] [amd64|arm64]` works for any host.

### Supported targets

Tier 1 (built, tested, and intended for release): `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`. The Linux glibc floor is **2.28**.

Tier 2 (built when a builder is available): Windows ARM64 and both musl targets.
Everything else — 32-bit, i686, Windows GNU, other Unixes — is explicitly out of scope.
Full contract in [`docs/TARGET-MATRIX.md`](docs/TARGET-MATRIX.md).

---

## 4. Quick start

Trailing slashes mean exactly what they mean in rsync: `src/` copies the *contents* of
`src`, and `src` copies the *directory itself* into the destination.

```bash
# Local copy
xs ~/Documents/notes/ /Volumes/Backup/notes/
```

```bash
# Push to a remote host over SSH
xs ~/Documents/notes/ freya:/srv/backup/notes/
```

```bash
# Pull from a remote host
xs rpi5:/srv/media/photos/ ~/Pictures/photos/
```

```bash
# See what would happen, without writing anything
xs -n --delete ~/Documents/notes/ freya:/srv/backup/notes/
```

```bash
# Mirror: also remove files the source no longer has
xs --delete ~/Documents/notes/ freya:/srv/backup/notes/
```

```bash
# Skip paths
xs --exclude 'node_modules/**' --exclude '*.tmp' ~/projects/ mars:/srv/projects/
```

```bash
# Compare by content hash instead of size and timestamp
xs --checksum ~/Documents/notes/ freya:/srv/backup/notes/
```

```bash
# Machine-readable event stream, one JSON object per line
xs --progress-json ~/Documents/notes/ freya:/srv/backup/notes/
```

**One end of every transfer must be local.** `xs freya:/a rpi5:/b` fails immediately with
`remote-to-remote sync is not supported in v1`. To sync between two remote hosts, drive it
from one of them — that host is then a local endpoint of its own transfer:

```bash
ssh freya 'xs /srv/media/ mars:/srv/media/'
```

---

## 5. The complete command reference

```
xs [OPTIONS] <SRC> <DEST>
xs [OPTIONS] <JOB>
```

Either `SRC` or `DEST` may be `[user@]host:path`. `~` and `~/sub` expand on the remote
side. A Windows drive letter (`C:\foo`, `C:/foo`) is always parsed as a local path, never
as `host:path`.

| Flag | What it does |
|---|---|
| `-n`, `--dry-run` | Plan everything, print one line per intended `create` / `update` / `delete`, write nothing. |
| `--delete` | After every transfer and verification succeeds, remove destination entries the source no longer has. Deletes are held until the end on purpose — a failed transfer never triggers a delete. |
| `--exclude <GLOB>` | Repeatable. Glob matched against the path *relative to the root*, using `globset` semantics, so `**` crosses directory boundaries. Excluded directories are pruned rather than walked. Using any filter disables the directory-clone fast path. |
| `--include <GLOB>` | Repeatable. Transfer matching paths, overriding a *later* exclude. First match wins, in command-line order. No `--include '*/'` needed. Local transfers only — see §7. |
| `--exclude-from <FILE>` | Read exclude patterns from FILE, one per line; `#` comments and blank lines ignored. Repeatable. |
| `--include-from <FILE>` | Read include patterns from FILE. Repeatable. |
| `--no-ignore-file` | Stop honouring per-directory `.xsyncignore` files, which are on by default. |
| `--explain-filter` | Print the rule that decided each excluded path, with its file and line. Pair with `-n`. |
| `--checksum` | Classify files by BLAKE3 content hash rather than by type + size + mtime. Digests are cached (see §7). |
| `--paranoid` | After writing a file, re-read it from disk and verify its hash. Also forces byte-level verification on the clone fast paths, which otherwise verify only that the operation succeeded and the source did not change. |
| `--streams <N>` | Number of parallel SSH data streams, 1–16. **Default 1.** |
| `--no-compress` | Turn off zstd compression entirely. |
| `--compress-level <L>` | zstd level, 1–22. Default 3. |
| `--transport <auto\|xsync\|rsync>` | Which remote engine to use. `auto` (default) prefers native xsync and falls back to the rsync protocol only if the remote `xs` is genuinely missing. |
| `--cloud-files <download\|skip\|error>` | What to do about cloud-provider placeholder files (macOS File Provider). Default `download`. |
| `--progress-json` | Emit JSONL events on stdout instead of progress bars. Schema in [`docs/progress-json-v1.md`](docs/progress-json-v1.md). |
| `-q`, `--quiet` | Suppress non-error output. Does **not** suppress the remote server's own stderr, which is drained and echoed locally. |
| `-e`, `--rsh <CMD>` | Remote shell command. Default `ssh`. |
| `--job <NAME>` | Run a saved job from the config file. Equivalent to passing the name as the only positional argument, but unambiguous when a path of the same name also exists. See §7. |
| `--config <FILE>` | Read jobs from `FILE` instead of the default search path. A file named here must exist. |
| `--list-jobs` | Print the jobs the config file defines, with their endpoints and flags, and exit. |
| `--server` | Hidden. Run as the remote receiver, speaking the protocol on stdin/stdout. This is what `ssh host xs --server /path` invokes; not for interactive use. |
| `--no-directory-clone` | Hidden, benchmarking only. Disables the directory-clone fast path. |
| `--version`, `--help` | Standard. |

### Exit codes

| Code | Meaning |
|---:|---|
| `0` | Everything completed. |
| `1` | The run failed. |
| `23` | **Partial failure** — some entries transferred and others did not, or policy (such as `--cloud-files=skip`) omitted work. Same number rsync uses for a partial transfer. |

Any non-zero exit deserves attention. A script that treats `23` as success will silently
accept an incomplete backup.

---

## 6. How it works, end to end

### The pipeline

Every transfer, local or remote, runs the same six stages. They are pipelined: work flows
between them through bounded queues, so the whole run has a memory ceiling that does not
depend on how many files there are.

**1. Discovery (scan).** A parallel directory walk built on the `ignore` crate's
`WalkBuilder`, with all its "standard filters" turned off — xsync does not skip hidden
files or honour `.gitignore`, because a sync tool that silently omits files is a bug, not a
feature. Symlinks are never followed. Each entry produces a `FileEntry`: its
relative path as raw bytes, kind, size, nanosecond mtime, permission mode, and a
**source fingerprint** (device + inode + size + mtime) used later to detect a file changing
underneath the transfer. Results flow into a bounded channel (default capacity 1,024) and
the scanner records its own high-water mark so queue pressure is observable.

**2. Destination index.** The destination is scanned into an index for comparison. Below a
memory budget (default 64 MiB) that index lives in memory; above it, it spills to a
per-run on-disk store, so a tree with tens of millions of entries does not exhaust RAM. The
index is per-run and is thrown away afterwards — xsync deliberately does **not** treat a
persistent index as filesystem truth.

**3. Planning.** Every entry is classified as **new**, **changed**, **unchanged**,
**type-replaced** (a file where a directory should be, or vice versa), or **extraneous**
(present at the destination, absent at the source). Default equality is type + size +
mtime; `--checksum` switches to BLAKE3 content comparison. Extraneous entries are recorded
but not acted on until every transfer has succeeded.

**4. Strategy selection.** Work is bucketed by size, because a 200-byte file and a 40 GB
file have nothing in common operationally:

| Bucket | Threshold | Handling |
|---|---|---|
| Small | ≤ 1 MiB | Coalesced into logical batches targeting 32 MiB or 8,192 files, whichever comes first. One metadata frame per batch. |
| Medium | ≤ 32 MiB | One whole-file work item each. |
| Large | > 32 MiB | Split into disjoint 16 MiB logical chunks that can be transferred independently, resumed independently, and striped across streams. |
| Local, same filesystem | any | Try a directory clone, then a per-file clone (only above 12 MiB), then fall back to a verified byte copy. |

Those thresholds are explicitly documented as **hypotheses under measurement**, not
protocol constants. Both peers must agree on them for a session, but they can change
between versions without changing the wire format.

**5. Dispatch.** Local I/O worker count and remote stream count are **separate controls**.
Local workers default to `available_parallelism()`. Small and medium work goes through one
shared bounded queue so a single slow worker cannot head-of-line block everything behind
it. Large-file ranges keep stable stream ownership where the transport requires it.

**6. Write and verify (the sink).** Every write goes to a deterministic staging file named
`.xsync.tmp.<hash>` beside the final destination, is verified, has its metadata applied,
and is then **atomically renamed** into place. A crash, a disconnect or a kill never leaves
a truncated file at the real pathname — it leaves a `.xsync.tmp.*` file, which is the
signature of an interrupted run. Directory metadata is applied last, deepest-first, because
writing a child bumps its parent's mtime and that has to be undone afterward.

### Source stability: not blessing a mixture of two versions

This is subtle and worth spelling out. If a file changes while you are reading it, you can
end up with the first half of version A and the second half of version B — and hash it,
and get a perfectly valid digest for a file that never existed.

xsync's `SourceReader` opens the file without following symlinks, checks the opened object
against the fingerprint recorded during the scan, reads, and then **re-checks the
descriptor and the pathname after the final read**. If anything moved, it retries once with
a fresh scan. A second change is reported as a named partial failure for that entry, and
the rest of the run continues.

### Path safety

Wire paths are an ordered sequence of relative components carried as **raw bytes**, not as
a UTF-8 `String`. Unix filenames that are not valid UTF-8 survive a round trip intact.
(Windows filenames are currently represented as UTF-8, which is a known limitation.)

The receiver rejects, before touching the filesystem: empty components, absolute or rooted
paths, `.` and `..`, NUL bytes, platform prefixes, duplicate destinations under the
receiver's own case and normalization rules, and **any traversal through a pre-existing
symlink**. That last one matters: the most recent hardening commit added an explicit rule
that an ancestor symlink is never resolved, because it would let an interrupted or
malicious run redirect writes outside the destination root. The final component may
legitimately be replaced — a file becoming a directory is a normal type replacement — but
an ancestor may not.

The same commit added a **source/destination overlap check**: `xs ~/a ~/a/b` is rejected
rather than being allowed to copy a tree into itself.

### Sparse files

A sparse file reports a large apparent size while occupying far fewer blocks; the
unwritten regions are holes. **xsync has no concept of a hole and will read and write
every zero.** A Docker VM disk on the development machine reports 3,996 GB apparent
against 140 GB allocated — a **28.6x** amplification that does not merely run slowly, it
exhausts the destination and fails after hours.

Sparse-aware transfer is planned; until it lands, xsync says so before starting work that
may not fit. Every planned file at or above 1 MiB is checked against its allocation, and
any occupying less than half its apparent size is reported by name with both figures and
the amplification, followed by a total. `--dry-run` shows the same, so the real cost is
visible before committing:

```
warning: disk.img: sparse source: 8.0 GiB will be read and written although it occupies
only 16.0 KiB (524288.0x amplification). xsync does not yet transfer holes; the
destination needs room for the larger figure.
warning: 1 sparse file(s): 8.0 GiB will be written to carry 16.0 KiB of real data —
8.0 GiB of it holes that xsync cannot yet skip.
```

These are warnings, not failures: a dense copy of a sparse file is wasteful and may not
fit, but it is not incorrect, and refusing would block a user whose destination has room.
Files below 1 MiB are never inspected, since a file smaller than a filesystem block
cannot be meaningfully sparse. On Windows nothing is reported — allocation is exposed
through `GetCompressedFileSize`, which the standard library does not wrap, and inventing
a number would be worse than silence.

### Destination path collisions

Two paths that are distinct on the source can be the *same* path on the destination.
APFS and NTFS are case-insensitive by default, and APFS additionally treats canonically
equivalent Unicode forms — `café.txt` written as U+00E9, and as `e` + U+0301 — as one
name. Publishing both keeps whichever wrote last, which is silent data loss.

xsync **probes the destination** before planning, by creating two names that differ only
in case and two that differ only in normalization, and observing whether each pair
collides. The behaviour belongs to the volume rather than the operating system — a macOS
volume can be formatted case-sensitive, and Linux can mount NTFS — so it is never
inferred from the platform. When the destination does not exist yet, its nearest existing
ancestor is probed instead, since the property comes from the volume.

If two source paths would land on one destination name, the run is **refused before
anything is written**, naming the collision and the reason. `--on-path-collision=skip`
omits every path involved in a collision instead, reporting each as a failure and exiting
with the partial-failure code. There is no mode that publishes one and discards the other
silently.

Non-UTF-8 names are compared byte-wise: they cannot be normalized, and a filesystem
storing raw bytes distinguishes them.

### Atomic publication and resume

For large files, the receiver records each verified byte range into a **durable resume
journal** under `$TMPDIR/xsync-resume-<16-hex>`, and flushes the staged bytes to disk
*before* the checkpoint is written, so a checkpoint can never acknowledge data that exists
only in the page cache. On restart, the journal is consulted, ranges already verified are
skipped, and only the uncheckpointed window is resent. If the source file's fingerprint has
changed, the old ranges are invalidated rather than combined with new ones.

Small and medium files are whole-file work items, so an interrupted run resends them. That
is a deliberate distinction: **safe restart** (never leaving a corrupt file) applies to
everything; **durable resume** (not resending verified bytes) applies to large files only.

---

## 7. Every feature, explained in plain English

### Whole-file transfer with BLAKE3 integrity

When a file needs to move, xsync sends the whole file. There is no rsync-style delta
algorithm in v1 — a one-byte change in a 4 GB file resends 4 GB. In exchange, the transfer
is simpler, has no round-trip-per-block cost, and hashes everything with BLAKE3 as it goes.

Metadata preserved: modification times, Unix permission bits, empty directories, and
symlinks as symlinks. Not preserved: hardlinks, ownership, ACLs, extended attributes,
resource forks, and sparse layout.

### The directory-clone fast path

On a copy-on-write filesystem, duplicating a file or a whole tree is a metadata operation:
both copies point at the same physical blocks until one of them is written. xsync uses this
whenever the source and destination are on the same filesystem and the destination subtree
is entirely absent.

- **macOS / APFS**: a direct `clonefile(2)` call. This is the one place xsync uses
  `unsafe` — see [Why there is one `unsafe` block](#why-there-is-one-unsafe-block) below.
- **Linux**: `cp --reflink=always`, which works on btrfs and on XFS with reflink enabled,
  and fails cleanly on ext4 and ZFS so xsync falls back to a normal copy.

Before any of this is attempted, xsync **probes the destination once per run** by cloning a
single one-byte file. On a filesystem without reflink support every clone attempt is doomed,
and the attempts are not cheap: measured on ext4, the clone machinery cost **65.8% of total
wall time** (0.572 s against 0.196 s with cloning disabled) for a corpus `rsync -a` copies in
0.323 s. One probe removes that entire class of wasted work.

Measured effect: cloning a 206 MB file locally is **5.0x faster** than `rsync -a`. On a
109,615-file corpus on APFS, the tree clone publishes the whole thing in **4.128 s** against
`rsync -a`'s 24.024 s — **5.8x faster**.

Per-file cloning is only attempted **above 12 MiB** (`FILE_CLONE_MIN_BYTES`), because a
five-repetition APFS measurement found the clone setup and validation cost more than a
plain verified copy below that: 0.502x at 4 MiB, 0.863x at 8 MiB, 1.130x at 12 MiB, 1.448x
at 16 MiB.

`--exclude` disables directory cloning, because a clone is all-or-nothing and cannot honour
a filter. `--paranoid` adds byte readback to a clone, which otherwise verifies only that
the operation succeeded and the source did not change during it (a clone has no incoming
byte stream to hash).

#### Why there is one `unsafe` block

The workspace sets `unsafe_code = "deny"`. `crates/xsync-core/src/clone.rs` carries the only
exemption, a single `#[allow(unsafe_code)]` on the function that calls `clonefile(2)`.

It is there because the safe alternative is **6.3x slower**. `/bin/cp -c -R` does not clone a
tree; it performs a *per-file* `COPYFILE_CLONE` and recurses. On a 109,615-file corpus,
`cp -c -p -R` took **23.610 s** while one `clonefile()` on the tree root took **3.766 s**, for
byte-identical output. Shelling out also costs a process spawn per clone. End to end this took
xsync's time on that corpus from 28.310 s to 4.128 s, turning a 1.19x loss against `rsync -a`
into a 5.8x win.

The block itself is three lines: two `CString`s and one FFI call whose arguments are two
NUL-terminated paths and a flag. It shares no memory with the callee and retains nothing after
it returns. `CLONE_NOFOLLOW` is passed so the call never resolves a symbolic link it was asked
to copy, matching the rest of the engine.

One behaviour differs from `cp -p -R` and is compensated for explicitly: **`clonefile` does not
preserve directory modification times.** Populating a cloned directory updates its own mtime,
and unlike `cp -p -R` nothing sets it back. xsync therefore reapplies every cloned directory's
mtime from the plan, deepest-first and with the subtree root last, before publishing the stage.
This was caught by the integration suite rather than by inspection — three tests comparing a
local sync against a push failed on exactly this difference.

The same question applies to Linux's `FICLONE` ioctl, which would remove the `cp` process spawn
there too. It has not been done: the measured prize on Linux is much smaller, and the reflink
probe above already removes the dominant cost on filesystems that cannot clone at all.

### Adaptive compression

zstd level 3 is **on by default**. Whether it is actually used is decided per logical
payload by compressing a bounded sample — 64 KiB for small payloads, scaling to 1 MiB for
large ones — and using compression only if the sample comes out at **95% or less** of its
input.

This is decided from the bytes, never from the filename. That turns out to matter: the
assumption that build artifacts are incompressible is simply wrong. Measured on real
corpora, `.o` files compress **7.8x** and JS/JSON **4.3x**, while a directory of `.cbz`
manga archives compresses to **1.00x** (the output is fractionally larger) and is correctly
skipped. On the synthetic compressible corpus, compression reduced wire bytes **713x**;
on the incompressible corpus it was correctly disabled and wire bytes were identical.

Compression is negotiated in the handshake through a capability bit. If either side does
not advertise `CAP_ZSTD`, the session uses no compression, and that decision is made before
any data frame is sent.

### `--checksum` and the hash cache

By default two files are considered equal if their type, size and mtime match. `--checksum`
replaces that with a BLAKE3 comparison of the actual content, which catches the case where
something rewrote a file without changing its size or restored a stale timestamp.

Hashing a whole tree is expensive, so digests are cached in a redb database at
`$XDG_CACHE_HOME/xsync/hashes.redb`, falling back to `~/.cache/xsync/hashes.redb`. The key
includes stable filesystem identity, size, mtime and change-time, and a cache hit is only
accepted after a stable metadata read. **The cache is an optimization, never an
authority** — corruption or a schema mismatch rebuilds it rather than being trusted.

This path had a serious performance bug that is worth recording because the fix is
instructive. `HashCache::hash_file` opened and committed a separate redb write transaction
per cache miss, and redb commits durably by default, so **every single file cost an
fsync** — 9 ms of pure blocking per file, making `--checksum` 63x slower than `rsync -ac`.
Digests are now buffered in memory and committed in batches (once per run for any tree
under 4,096 files, and again on drop), at `Durability::Eventual` rather than `None`,
because redb only reclaims pages above `None` and committing at `None` would grow the cache
file without bound. Result: 4.09 s → **0.21 s**, a 19x improvement, with the paired ratio
against `rsync -ac` improving 30x from 0.016 to 0.487.

### `--paranoid`

Re-reads every written file from disk and verifies its BLAKE3 hash before considering it
done. This is the answer to "I do not trust this disk, this cable, or this filesystem." It
roughly doubles destination I/O. It also supplies real byte verification to the clone fast
paths.

### `--delete`

Removes destination entries that no longer exist at the source. Deletes are deferred until
every transfer and verification has succeeded, so a mirror is never pruned on the strength
of a run that failed halfway through. `--dry-run --delete` prints the exact set of
`delete <path>` lines it would perform.

### Filters: `--exclude`, `--include`, and friends

Five flags, one model:

| Flag | Effect |
|---|---|
| `--exclude <GLOB>` | Skip matching paths. Repeatable. |
| `--include <GLOB>` | Transfer matching paths, overriding a *later* exclude. Repeatable. |
| `--exclude-from <FILE>` | Read exclude patterns from a file, one per line. Repeatable. |
| `--include-from <FILE>` | Read include patterns from a file. Repeatable. |
| `--no-ignore-file` | Stop honouring per-directory `.xsyncignore` files. |
| `--explain-filter` | Print the rule that decided each excluded path. |

**The rule is: first match wins, in the order you wrote them.** Nothing matching means the
path is transferred — rules subtract from "copy everything". The order is the order on the
command line, *including across different flags*, and a `--exclude-from` file's rules expand
exactly where its flag appeared.

```bash
xs --include 'docs/guide.md' --exclude '*.md' src/ dst/
```

keeps `docs/guide.md` and drops every other `.md`. Reverse the two flags and the exclude
matches first, so the guide goes too. That is the whole model.

#### Three things this deliberately does differently from rsync

**1. No `--include '*/'` incantation.** In rsync this is the classic silent failure:

```bash
rsync -a --include 'docs/**' --exclude '*' src/ dst/   # transfers nothing
```

`docs` matches `*`, so rsync prunes it before the include rule is ever consulted, and the
command copies nothing while reporting success. xsync walks a directory whenever an include
rule *could* match something beneath it, computed from the rules themselves, so the obvious
command does the obvious thing:

```bash
xs --include 'docs/**' --exclude '*' src/ dst/         # transfers docs/
```

The directory `docs` itself is still excluded — it just gets walked. Over-descending costs
one `readdir`; under-descending loses files silently, so the trade is not close.

**2. An explicit rule about a path beats an inherited one.** A path under an excluded
directory is excluded — that is what makes pruning and per-path evaluation agree. But a rule
naming the path itself wins over one that only named an ancestor. This is what makes
"exclude the tree, keep one file in it" expressible at all.

**3. Per-directory ignore files are *weaker* than the command line**, not stronger. See
below.

#### `.xsyncignore`

A `.xsyncignore` file applies to the directory holding it and everything below, with
patterns relative to that directory — the `.gitignore` model. Blank lines and `#` comments
are ignored, `+ ` and `- ` prefixes override the default action, and `!pattern` means
include (because enough people will reach for the gitignore spelling that treating it as a
literal filename would be a trap).

```
# src/.xsyncignore
target
*.log
!logs/keep.log
```

They are honoured by default and are **always weaker than command-line rules**, so a command
line can override a tree's own opinion and never the other way round:

```bash
xs --include 'logs/**' src/ dst/    # transfers logs/ even though .xsyncignore drops *.log
```

This is the opposite of what falls out of delegating to an off-the-shelf ignore walker,
which prunes during the walk and therefore beats every rule the user typed. Getting the
direction right is the reason xsync reads these files itself.

The `.xsyncignore` file is itself transferred, like `.gitignore` is committed: the
destination should be a faithful copy of the source, including its opinions.

#### `--explain-filter`

Names the rule that removed each path, and where it was written — file and line for rules
files and ignore files:

```
$ xs -n --explain-filter --exclude-from rules.txt src/ dst/
filtered logs/build.log: excluded by '- *.log' (.xsyncignore:3)
filtered docs/api/ref.md: excluded by '- *.md' (rules.txt:2)
filtered target: excluded by '- target' (.xsyncignore:2)
filtered target/debug/app: excluded by '- target' (.xsyncignore:2), which excluded the parent 'target'
```

It makes the walk visit what it would otherwise prune, so it costs more than a normal run —
which is why it is a flag and not the default. Pair it with `-n` to inspect a filter before
trusting it.

#### The remote limitation, stated plainly

The v1 wire carries a flat list of exclude patterns and nothing else. So:

- `--exclude` and `--exclude-from` work for remote transfers. Rules-file patterns are folded
  into the list that crosses the wire, so a rules file is not silently dropped.
- **`--include` is refused for remote transfers.** An include rule's meaning is its position
  relative to the excludes; sending the excludes alone would transfer a *larger* set than
  asked for, silently. Failing is the only safe answer:

  ```
  xs: --include is not supported for remote transfers yet: the v1 wire carries only
  exclude patterns, and sending those alone would transfer more than you asked for.
  ```

- **`.xsyncignore` files are not honoured when the source is remote**, because the far side
  does the walking. This warns rather than fails: an unseen ignore file cannot make the
  transfer wider than the explicit rules already allow.

Carrying the full ruleset would need a capability bit and a server-side filter; the wire
already reserves un-masked capability bits for exactly this kind of addition.

### `--dry-run`

Prints one line per planned mutation and writes nothing. Because deletes are the
irreversible operation, dry-running a `--delete` is the recommended habit.

### Named jobs and the config file

Nobody retypes an exclude list and a destination path daily. They write a shell alias — and
from that moment the tool's own `--dry-run`, logging and error messages never see the real
configuration. A **job** moves that configuration inside xsync, where it can be listed,
dry-run, and reported on.

```toml
# ~/.config/xsync/config.toml

[jobs.documents]
src = "~/Documents/"
dest = "mars.local:/backup/documents"
description = "nightly documents backup"
exclude = ["*.tmp", ".DS_Store"]
checksum = true
delete = true

[jobs.photos]
src = "~/Pictures/"
dest = "/Volumes/Archive/Pictures"
streams = 4
```

Three ways to run one:

```bash
xs documents
```

```bash
xs --job documents
```

```bash
xs --config ./project.toml --job documents
```

`xs --list-jobs` prints what is defined, with each job's endpoints and the flags it sets.
`xs -n documents` dry-runs it, so a saved job can be inspected before it is trusted.

**Search path**, in order. The first file that exists wins:

1. `--config FILE` — an explicit path. If it does not exist, that is a fatal error, not a
   fall-through: running a *different* configuration than the one named would be worse
   than refusing.
2. `$XSYNC_CONFIG` — a file named by the environment.
3. `$XDG_CONFIG_HOME/xsync/config.toml`, or `~/.config/xsync/config.toml`.
   On Windows, `%APPDATA%\xsync\config.toml`.

XDG is used on macOS too, rather than `~/Library/Application Support`. A config file is
something people edit by hand and copy between machines, and a path that is identical on
the laptop and on the server is worth more here than platform purity.

**Precedence is flag > job > built-in default**, decided by *what the user actually typed*
rather than by whether a value differs from its default. That distinction matters:
`--transport auto` is indistinguishable from an untouched default by value alone, and
treating it as absent would let the job silently win an argument the user explicitly made.
A `--exclude` on the command line *replaces* the job's list rather than adding to it, so a
one-off run is never quietly broadened by patterns saved months ago.

**One asymmetry, stated rather than hidden.** A boolean a job turns on cannot be turned off
from the command line, because there is no `--no-delete`. A job with `delete = true` is a
job that always deletes; if you want it optional, leave it out of the job and pass
`--delete` when you mean it.

**A malformed config is fatal at startup and never partially applied.** The whole file is
parsed and every job validated before a single value reaches the run, so a config whose
*second* job is broken will not run the first. Unknown keys are errors:

```
xs: invalid config 'config.toml': TOML parse error at line 4, column 1
  |
4 | excludes = ["*.tmp"]
  | ^^^^^^^^
unknown field `excludes`, expected one of `src`, `dest`, `description`, `exclude`, ...
```

That plural is the realistic typo, and quietly ignoring it would mean a backup that copies
files the user believed were excluded. Endpoints are parsed and paired at load time too, so
a remote-to-remote job is refused when the config is read rather than when it is run.

**Ambiguity is refused, not guessed.** If `xs backup` could mean either the saved job
`backup` or the directory `./backup` that also exists, xsync stops and says so:

```
xs: 'backup' is both a saved job and an existing path; use '--job backup' to run the
job, or './backup' to name the path
```

Choosing for you could copy the wrong tree.

A leading `~/` in `src` or `dest` is expanded against `$HOME`. Only a leading `~/`, and
never in a remote spec: in `mars:~/data` the tilde belongs to the remote shell, and
expanding it locally would send the wrong path.

### `--progress-json`

Emits JSON Lines on stdout: one object per event, each with `type`, `schema_version: 1`,
and `timestamp_unix_nanos`. Event types are `phase`, `started`, `planned`,
`cloud_placeholders`, `action`, `metrics`, `transferred`, `skipped`, `deleted`, `warning`,
`failed`, and `done`.

The `done` event carries the complete summary: logical, physical and wire bytes;
transferred / skipped / deleted / failed counts; worker and stream counts; clone and copy
counts; resume and retransmission counters; transport identity; negotiated wire version;
which options were mapped and which guarantees are unavailable; and the checksum and
compression algorithms actually used. Phase timing comes from paired `phase` events —
record the timestamp on `started: true` and subtract it from the matching `started: false`.

Unknown fields are forward-compatible; changing the meaning of an existing field requires a
new schema version. The terminal progress UI and the JSONL renderer consume the exact same
event stream, so they can never disagree.

### The three byte counters

Reporting distinguishes three quantities, which is unusual and useful:

- **logical bytes** — the size of the files as the user thinks of them;
- **physical bytes** — bytes actually moved through the streaming read/write path (a clone
  reports zero, because it moved none);
- **wire bytes** — application-protocol bytes actually written to the transport, after
  compression.

A local clone of a 200 GB tree correctly reports 200 GB logical, 0 physical, 0 wire. This
is what makes it possible to tell "we went fast because we compressed well" apart from "we
went fast because we did not copy anything."

### `--streams`

Up to 16 parallel SSH data sessions. **The shipping default is 1**, and that is an
evidence-backed decision, not conservatism: two to four streams often helped and eight
sometimes regressed badly, in a way that depended on the destination filesystem rather than
on core count. More importantly, the measured win from framing the protocol properly was
20–80x while the additional win from parallel streams was 1.0–1.6x — parallelism was never
where the headline lived. The gate for raising the default requires a paired improvement on
at least two materially different remote filesystems with no material regression elsewhere.

Multi-stream workers are coordination-free in memory but share the receiver's durable
staging and checkpoint state, and no two streams ever write the same byte range. A stream
requests a data-only receiver session with the `CAP_DATA_ONLY` capability bit; a server
that does not understand the bit still answers as an ordinary single sink, so a new client
against an old server degrades to one stream instead of failing.

### `--cloud-files`

On macOS, files that a cloud provider has evicted to the network are marked with a File
Provider extended attribute. Reading one silently triggers a download, which can turn a
"local" sync into hours of network transfer. `--cloud-files` chooses: `download` (default,
materialize them), `skip` (omit them and report a partial result), or `error` (refuse the
job). Detection is macOS-only; the policy flags require detection to be available.

### The rsync-protocol fallback

If `--transport=auto` and the remote host has no `xs` binary at all, xsync can fall back to
speaking **the rsync wire protocol itself**. This is a native Rust implementation of the
protocol — it does not require a local `rsync` executable, and it is not a disguised
`tar` or `sftp` copy. It launches the remote `rsync --server` and talks to it directly.

The boundaries are narrow and deliberate:

- **Push and pull.** Both directions speak to the remote GNU sender or receiver
  directly; neither requires a local `rsync` executable or a remote `xs`.
- **GNU rsync advertising protocol 32 only** (3.4.x / 3.5.x). macOS's `/usr/bin/rsync` is
  openrsync protocol 29 and is rejected with an explicit message. A newer GNU rsync that
  negotiates down to 32 is accepted.
- **Rejects** `--streams > 1`, `--delete`, `--paranoid`, `--checksum`, and
  `--compress-level`; compression is simply not offered on this path.
- Supported over this path: regular files, nested and empty directories, symlinks, modes,
  mtimes, quick-check skipping, whole-file transfer, raw Unix path bytes, and type
  replacement.

Crucially, **the fallback is never used to paper over a failure**. Authentication failure,
host-key failure, protocol corruption, version mismatch, or a native transfer that already
started mutating the destination are all real errors. Retrying them through a different
engine could hide a security or correctness problem. The fallback fires only when the
remote xsync is *genuinely absent*. And xsync never uploads or installs an executable just
because `--transport=auto` was selected.

Every event and the final summary identify `transport=rsync`, the negotiated protocol
version, the remote implementation, which options were mapped, and which xsync guarantees
are unavailable — multi-stream striping, BLAKE3 frame verification, xsync atomic staging,
and xsync checkpoint resume are all explicitly reported as not applicable.

Full contract: [`docs/rsync-wire-v1.md`](docs/rsync-wire-v1.md).

### SSH, and what xsync deliberately does not touch

xsync runs ordinary OpenSSH: `ssh host 'PATH="$HOME/.local/bin:$PATH" xs --server /path'`.
It does **not** create, require or modify a `ControlMaster` socket, and it does not touch
host-key checking, authentication, or agent policy. That is a recorded design decision, not
an oversight — those are the user's security settings and duplicating them would weaken
them.

The practical consequence: key-based authentication is effectively required, because xsync
spawns a non-interactive SSH session and cannot answer a password prompt.

The remote command is quoted safely; a destination path containing `'; touch pwned; echo '`
is passed through as a literal path, and there is a test asserting exactly that.

---

## 8. The wire protocol

The protocol is versioned, language-neutral, and specified independently of the Rust types
that implement it — Rust enum ordering and the choice of serialization library are
explicitly *not* the wire contract. The full specification is [`protocol.md`](protocol.md).

### The envelope

Every frame starts with a fixed 32-byte little-endian header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic, ASCII `xsn1` |
| 4 | 2 | header length, exactly 32 |
| 6 | 2 | reserved, zero |
| 8 | 4 | protocol version |
| 12 | 1 | message type |
| 13 | 1 | flags (bit 0 = zstd) |
| 14 | 2 | reserved, zero |
| 16 | 4 | encoded payload length |
| 20 | 4 | decoded payload length |
| 24 | 8 | unique message ID |

### Fail-closed, everywhere

This is the property the protocol is actually designed around. A v1 receiver rejects — it
never ignores — an unknown magic, version, header length, reserved value, message type,
flag, enum value, duplicate message ID, overlapping byte range, trailing byte, or length
violation. Length arithmetic is checked before anything is allocated or read.

Concretely bounded: payloads at 16 MiB encoded and 16 MiB decoded; data segments and
large-file ranges at 8 MiB; encoded paths at 1 MiB; error messages at 64 KiB UTF-8;
collection counts at 65,536 records per frame; the default unacknowledged window at 32 MiB.
A compressed frame declares its decompressed size in advance and is decompressed only to
that declared, capped length — so a zip-bomb frame cannot make the receiver allocate.

Because collections are capped, a logical 32 MiB batch or a 16 MiB chunk is represented as
*several* bounded frames, never one oversized one. Message IDs are tracked as a bounded set
of disjoint received ranges, so duplicates are rejected without imposing a frame-count
limit on a long transfer; a hostile stream with more than 1,048,576 disjoint ranges is
rejected by the session budget.

Protocol decoding never writes to the destination. The sink consumes only fully decoded,
separately verified operations.

### The thirteen v1 message types

Type bytes are frozen: `1` Handshake, `2` SessionConfig, `3` FileBatch, `4` FileSegment,
`5` LargeFilePrepare, `6` LargeFileRange, `7` LargeFileFinish, `8` Metadata, `9` Scan,
`10` Stats, `11` Ack, `12` Error, `13` ResumePage.

### Capability negotiation

`Handshake.capabilities` is a `u32` bitmap that is deliberately **un-masked**: a receiver
that does not recognize a bit simply ignores it. That is the one designed growth point in
an otherwise fail-closed protocol.

| Bit | Name | Meaning |
|---:|---|---|
| 0 | `CAP_DATA_ONLY` | Request a data-only receiver session, used by multi-stream striping |
| 1 | `CAP_ZSTD` | zstd payload support |
| 2 | `CAP_BROWSE_V2` | Endpoint implements the v2 browse message set |
| 3 | `CAP_VERSION_NEGOTIATION` | Endpoint understands the v2 handshake contract |

### Pipelining

An early implementation blocked for an acknowledgement after every frame, which meant every
small file cost two fully serialized network round trips and the negotiated 32 MiB window
was never used. Both directions now write frames without stopping, draining replies on a
bounded window of 256 outstanding frames. That bound is not arbitrary: an unbounded window
*deadlocks*, because the receiver blocks writing acknowledgements once its own pipe buffer
fills, and 256 keeps the peer's pending acknowledgements near 10 KiB, which fits inside any
ordinary pipe or SSH channel buffer. Fixing this made a 1,000-entry deep tree over SSH
**25.5x faster** — 8.73 s to 0.34 s.

---

## 9. Protocol v2 and the browse surface

xsync's wire format has picked up a second consumer. A separate project, `f2`, is adopting
xsync as its remote transport instead of maintaining its own Go agent, which means **one
wire format with two owners**.

That project is a file *browser*, and a browser needs something a sync engine does not
have. A sync session is one long planned operation over a whole tree. A browser issues a
stream of small, unrelated, latency-sensitive requests — list this directory, now that one,
rename this, read that file — over a connection that stays open as long as the window does.
v1's `Scan` message is a whole-tree scan feeding a diff, not an interactive listing.

Because v1 is fail-closed, a new message type cannot be added compatibly. So the browse
surface **is** protocol v2.

### Negotiation, and why a v2 client can still talk to a v1 server

The opening handshake is always sent in a **v1 envelope** with the frozen v1 payload, so a
v1 decoder accepts it without a compatibility exception. Both endpoints then compute the
same result from the two capability bitmaps:

```text
v2 = local has CAP_VERSION_NEGOTIATION and CAP_BROWSE_V2
  && remote has CAP_VERSION_NEGOTIATION and CAP_BROWSE_V2
selected_version = 2 if v2 else 1
```

The selection is committed exactly once, before `SessionConfig` and before any data frame.
There is no probe, no retry, and no mid-session fallback. A v1 selection is **never**
inferred from a v2 decode failure — that would be a security hole, not a compatibility
feature. A v2 session that receives a malformed v2 frame fails closed.

Result: a v2 client against a v1 server does ordinary v1 push and pull with browse reported
unavailable. A v1 client against a v2 server gets only v1 frames and never learns the
server supports more.

The negotiation is observable. A `protocol-negotiated` event reports `selected_version`,
`remote_capabilities`, `common_capabilities`, and `browse_available` before the first
selected-version frame is sent, so a JSON consumer can disable browse controls in its UI
without parsing human-readable output.

Full contract: [`v2handshake.md`](v2handshake.md).

### What v2 adds

Message types 14–35, all implemented in `crates/xsync-core/src/protocol_v2.rs` with
byte-exact conformance vectors in `protocol-v2-vectors/`:

| Area | Messages | Behaviour |
|---|---|---|
| Browse | `ListRequest` / `ListPage`, `StatRequest` / `StatResponse` | One directory at a time, paged with opaque server-issued cursors, page size 1–65,536. Stat can optionally include a BLAKE3 digest, but only for a regular file. |
| Session | `CancelRequest`, `Keepalive` / `KeepaliveAck`, `BrowseError` | Cancellation is scoped to one request. Browse errors are per-request and do not kill the session unless the error is a framing or bounds violation. Error codes are frozen: 1 cancelled, 2 invalid request, 3 permission denied, 4 unavailable, 5 internal. |
| Mutation | `RenameRequest`, `CreateDirectoryRequest`, `DeleteRequest` + responses | Rename refuses an existing destination and uses the filesystem's own atomic rename. Mkdir creates exactly one directory and does not create parents. Delete is recursive, **never follows symlinks**, emits one `DeleteProgress` per attempted item including failures, and always sets `irreversible=true` with every failed path and its signed platform errno. |
| Single-file transfer | `FetchRequest` / `FetchStart` / `FetchChunk`, `PublishRequest` / `PublishReady` / `PublishChunk` / `PublishResponse` | Fetch uses the same stable-source-read contract as sync and sends metadata plus digest up front. Publish is a **compare-and-swap**: the write is accepted only if the remote file's current size, mtime and filesystem identity still equal what was fetched. A mismatch returns `changed` plus the current identity, so a browser cannot silently clobber someone else's edit. |

The role byte gains value `3` for a long-lived session, which requires v2 negotiation and
the browse capability bits and never enters the v1 sync state machine.

### The connection model

One ordinary `ssh host xs --server <root>` process per remote pane, kept open for the
lifetime of the pane, with application keepalives sent more often than the deployment's SSH
`ClientAliveInterval` or any intervening NAT idle timeout.

There is no resumable session identity. A dropped link is reported as `PeerDisconnected`
and the client must reconnect and re-handshake. What must be redone is spelled out in
[`docs/browse-connection-model.md`](docs/browse-connection-model.md): every list from page
one (cursors do not survive), any mutation whose terminal response was not received — and
the client should *inspect remote state* rather than assume whether the syscall landed —
any fetch that did not verify its digest, and any publish that did not get a terminal
response, re-fetching identity first so it does not overwrite a newer edit.

An in-flight sync is unaffected by browse activity. It has its own durable journal and
chunk identity, so a separate link drop still resumes.

### The library API

The browse surface is **not exposed on the CLI**. It is a Rust API: `probe_session`,
`BrowseSession`, and typed helpers `list`, `stat`, `cancel`, `keepalive`, `rename`,
`create_directory`, `delete`, `fetch`, and `publish` in `xsync_core::server`. `probe_session`
returns `ProbeStatus::Ready` or `ProbeStatus::OlderPeer { selected_version }`, and
`into_browse_session()` continues directly into the session without a second connection or
handshake.

---

## 10. What xsync does not do

Being explicit about this is the point.

### No delta transfer

A one-byte change in a 4 GB file resends 4 GB. This is the single biggest functional gap
against rsync and is deferred to v2 behind a network-versus-disk cost model.

### No remote → remote

`xs hostA:/x hostB:/y` is rejected at argument parsing. The workaround is to run xsync
*on* one of the hosts, which makes it a local endpoint of its own transfer. This is honest
but it does fail the case where only a third machine holds credentials for both ends.

### Metadata not preserved

Hardlinks, ownership (uid/gid), ACLs, extended attributes, macOS resource forks, and
device/special files. Only mtimes, Unix permission bits, empty directories and symlinks
survive.

xsync says so before it starts, rather than leaving you to discover it. The preflight pass
counts, over the files actually being transferred:

- **Hardlinked files** — entries whose inode has more than one name. When two names for the
  same inode are both in the transfer, the destination gets two independent copies, and the
  extra bytes that costs are reported. A single name for a linked inode costs no extra
  space; only the link relationship is lost, and that is reported separately.
- **Extended attributes** — resource forks, Finder info, quarantine flags, Spotlight
  metadata. `com.apple.provenance` is deliberately not counted: recent macOS stamps it on
  essentially every file it creates, so counting it would fire the warning on every run and
  turn it into noise.
- **Foreign ownership** — entries owned by a different user or group than the one xsync is
  running as, whose copies will belong to the running user. The comparison is made against
  a file xsync actually creates in the destination, so it answers "who will own the copies"
  rather than inferring it.

The totals appear in the run summary and as `dropped_hardlinked_files`,
`dropped_hardlink_extra_bytes`, `dropped_xattr_entries` and `foreign_owner_entries` in the
`finished` JSON event. When nothing is being dropped, nothing is printed.

Two limits are worth stating plainly:

- **ACLs are not detected.** Counting them needs `libacl` on Linux and the `acl(3)` family
  on macOS, neither of which is a dependency today. A tree with non-trivial ACLs and no
  other dropped metadata reports nothing. This is a gap, not a guarantee.
- **`--dry-run` does not check ownership**, because answering that question requires
  creating a file in the destination and a dry run must not write there. It says so when it
  reports anything else. Hardlink and xattr counts are identical between a dry run and the
  real run.
- **Push to a remote host does not check ownership either**: uids on the far side mean
  something different, so the comparison would produce a warning nobody could act on.

The cost is one `listxattr` and one `stat` per transferred file. Measured on congress-100k
(109,615 files, local APFS, paired reps, same binary behind a temporary toggle): no
measurable difference at 10k files; **+11.7%** at 100k, of which roughly half is the `stat`
and half the `listxattr`. The `stat` is a duplicate of one the scanner already makes, but
plan entries reach the preflight reconstructed from the index encoding, which carries
identity, size and times but not ownership or link count — and that encoding is shared with
the frozen v1 wire format. Carrying those three fields through it would remove about half
the cost and is the obvious follow-up.

### Sparse files are catastrophic

xsync has no concept of a hole. A sparse file is read, hashed, transferred and written at
its **logical** size. A measured 130 GB sparse VM image reads and writes **3.7 TB** — 28.6x
write amplification — and cannot complete on any destination volume available for testing.
**Exclude VM images, thin-provisioned disks and `.img` files** until this is fixed. Note
that the information needed to fix it is nearly free: that image's complete 17,145-extent
map enumerates in 1.02 seconds.

xsync will at least **tell you** before it starts: every planned file at or above 1 MiB is
checked against its allocation, and any that is substantially sparse is named with both
sizes and its amplification, in the run and in `--dry-run`. See
[Sparse files](#sparse-files) in §6. The transfer is still dense; only the surprise has
been removed.

### Windows works as a client, not as a server

Windows compiles, is covered by CI on every commit, and can *initiate* a transfer to a
Linux host — the remote shell is then `bash`, which parses the command correctly.

It cannot be the *remote end*, for two remaining reasons:

1. **The remote command is POSIX-shell shaped.** xsync builds
   `PATH="$HOME/.local/bin:$PATH" 'xs' '--server' '/path'` and hands it to
   `ssh HOST '<command>'`. Under the Windows OpenSSH default shell (`cmd.exe`) that is not
   a valid command line. `-e/--rsh` does not help, because the POSIX form is appended
   whenever a host is present. It would work if the Windows sshd default shell were set to
   a POSIX shell such as Git Bash's `bash.exe`.
2. **The missing-binary probe does not recognize cmd.exe.** `is_missing_xsync_stderr`
   matches `xs: command not found`, `xs: not found`, and exit 127; cmd.exe says
   `'xs' is not recognized as an internal or external command`, so a missing remote binary
   surfaces as a raw transport error instead of a clean fallback.

(The third historical blocker — a bundled launcher that invoked a nonexistent `xsync.exe` —
is fixed. `xsync-server.cmd` now invokes `xs.exe`, and `build.rs` asserts it at compile
time.)

Also on Windows: filenames that are not valid UTF-8 are unsupported, because the v1 scanner
represents Windows filenames as UTF-8 protocol paths. Unix keeps full raw-byte fidelity.

### Content verification

Complete `FileSegment` messages carry a sender-computed BLAKE3 digest, and the receiver
checks that digest before publishing the staged file. This required the native sync wire
version bump to v2. Large-file pulls still verify each received range, but a complete-file
digest is not yet exchanged for the final striped publication; see `BUGS.md`.

### Everything is one-shot

There is no daemon, no scheduler, no watcher, and no config file. Every transfer is a
command you run.

---

## 11. Performance, with the actual numbers

Every number below comes from a checked-in report under `benches/results/`. Each is a
paired comparison against an rsync baseline measured in the same run, with rotated method
order, five or more repetitions, and an independent manifest oracle verifying the
destination after every single run. Rows whose median absolute deviation exceeds 15% of the
median are marked *noisy* and are explicitly **not** treated as evidence.

### Where xsync wins

Synthetic smoke corpora, paired ratio against `rsync -a` in the same run:

| Case | Route | Ratio |
|---|---|---:|
| one-large-file, initial copy | same-volume | **2.54x** |
| mixed, no-op second sync | same-volume | **1.98x** |
| compressible, initial copy | same-volume | **1.49x** |
| mixed, content churn | pipe | **1.47x** |
| incompressible, initial copy | same-volume | **1.40x** |
| mixed, type replacement | same-volume | **1.30x** |

Real corpora, local, same volume:

| Case | `rsync -a` | xsync | ratio |
|---|---:|---:|---:|
| single 206 MB `.cbz` | 0.35 s | **0.07 s** | **5.0x** |
| congress-10k, no-op re-sync | 0.86 s | **0.45 s** | **1.91x** |

Over real SSH to a Linux host, xsync is at or slightly above parity:

| Case | `rsync -a` | xsync | ratio |
|---|---:|---:|---:|
| flat-small, no-op | 0.2334 s | 0.2275 s | **1.03x** |
| one-large-file, initial copy | 0.8820 s | 0.8783 s | **1.03x** |
| deep-small, initial copy | 0.2993 s | 0.3429 s | 0.87x |
| compressible, initial copy | 0.7957 s | 0.7277 s | 1.10x *(noisy)* |
| incompressible, initial copy | 0.4747 s | 0.5958 s | 1.09x *(noisy)* |

Wire bytes, pipe route:

| Corpus | xsync | `--no-compress` | ratio |
|---|---:|---:|---:|
| compressible | 2,944 | 2,098,816 | **713x smaller** |
| incompressible | 2,098,816 | 2,098,816 | 1.00 — correctly skipped |
| mixed | 915,733 | 1,794,073 | 1.96x smaller |

### Where xsync loses, and by how much

This is the headline problem, measured on `congress-10k` — 11,280 real text files totalling
112 MB, local same-volume APFS, initial copy:

| | wall | user | sys | total CPU |
|---|---:|---:|---:|---:|
| `rsync -a` | 3.75 s | 0.32 s | 4.58 s | 4.90 s |
| `xsync` | 7.20 s | 7.52 s | 21.37 s | **28.89 s** |

xsync is **1.92x slower** in wall time and burns **5.9x the CPU**, of which 21.4 s is
*system* time — 1.9 ms of kernel time per file against rsync's 0.41 ms, a 4.6x gap.

And it does this **while copying zero bytes**. The APFS clone path engaged and the run
reported `0 physical`. With byte movement already eliminated, the entire remaining cost is
per-file syscall volume.

A fresh five-repetition rerun after the first round of optimizations still measured a 0.515
paired wall ratio (xsync median 5.977 s, rsync median 3.215 s). The same shape shows up on
the synthetic `deep-small` corpus locally: **0.578x**, with 2.46 s of CPU against rsync's
0.23 s — a 10.6x CPU gap that is compute-bound, not round-trip-bound.

> **Read that figure as a `congress-10k` result, because it is one.** Every measurement in
> the 0.515/0.534 family was taken on `congress-10k` (10,961 files). On `congress-100k`
> the same comparison **inverts**: R0a measured xsync **5.79 s** against `rsync -a`
> **25.72 s**, a paired ratio of **4.34x** in xsync's favour (2026-08-31, three reps,
> MAD ≤ 2.3%).
>
> The reason is that the two tools scale differently, not that one measurement was wrong.
> xsync's local cost is nearly flat in file count — **5.29 s at 10k, 5.79 s at 100k** —
> because the clone path does O(1) work per cloned subtree. rsync's is linear: **2.82 s to
> 25.72 s** for 10x the files. The crossover sits somewhere between 10k and 100k entries.
>
> So the per-file syscall analysis below is sound *at small scale*, and the conclusion that
> xsync's advantage comes from doing categorically less work is exactly right — that is
> precisely what the flat curve shows. What does not follow is that xsync is slower locally
> in general. **A current `congress-10k` local re-measurement is missing**; the 10k figures
> predate the clone work, so whether the loss still exists at that size is unknown.

**The conclusion this drives:** xsync does not have a bytes problem or a hashing problem.
It has a **per-file syscall volume problem**, and its durable advantages come from doing
categorically less work (cloning a subtree, skipping compression on incompressible data,
resuming instead of restarting) rather than from doing the same work faster.

### Fixes already landed, with their measured effect

| Fix | Before | After | Gain |
|---|---:|---:|---:|
| Small-file batching + pipelining (SSH, 1,000 entries) | 8.731 s | 0.343 s | **25.5x** |
| Hash-cache fsync-per-file (`--checksum`, content churn) | 4.0888 s | 0.2146 s | **19x** |
| Cloud-placeholder gate (`congress-10k` CPU) *(not gate-able — see §14 Phase 0.2)* | 29.388 s | 7.013 s | **4.2x** |
| Read buffer sized to `min(file_size, 64 KiB)` | — | — | landed |
| Skip clone attempt below 12 MiB | — | — | landed |

Correctness bugs the same matrix caught and fixed: directories classified *unchanged* had
their mtime bumped by the kernel when a child was rewritten and nothing restored it;
type-replaced directories were never created on the remote path because the push client
sent only `directories.new`; and the first mtime fix missed replaced directories' parents.
Each has a regression test that fails without the fix.

One optimization was measured and **not** claimed: caching the per-file temp-path hash
produced 3.070 s cached against 2.993 s uncached — inside the noise policy, no useful speedup. It
is retained for correctness but is explicitly not a performance win. That is the standard
this project holds itself to.

### What has not been measured

Everything above is `smoke` tier — the largest synthetic cell is 513 items and 1.77 MB, at
which scale process startup is a meaningful share of the measurement. The 100k-entry
regression tier and the full tier have not been run. There is no checked-in gate baseline
yet. There is no bandwidth-limited or genuinely cold-cache route, which means the regime
where compression and dedup actually dominate has never been measured at all — every route
so far has been a LAN or a local pipe with a warm cache, where wire bytes were never the
constraint.

---

## 12. The benchmark harness

`xsync-bench` is a separate workspace package that **deliberately does not depend on
`xsync-core`**, so its filesystem oracle cannot reproduce an engine scanner bug. If both
the engine and the verifier shared a walk implementation, they would share its blind spots.

### Deterministic corpora

Seven synthetic classes — `flat-small`, `deep-small`, `zero-byte-storm`, `mixed`,
`compressible`, `incompressible`, `one-large-file` — each preparable in seven workload
states: `initial-copy`, `no-op-second-sync`, `content-churn`, `metadata-only-churn`,
`type-replacement`, `delete`, and `interrupted-resume`. Two generations with the same
schema, seed, class and sizing produce identical manifest digests.

These are now documented as **legacy for performance purposes** and retained as the
correctness fixtures. They were retired from performance work for three measured reasons:
wrong shape (synthetic flat trees enumerate ~10x faster than real trees, which are
directory-open bound), wrong scale, and they hid a 5.9x CPU overhead that is unmissable at
real scale.

### Real corpora

Performance is now tuned against four live corpora referenced read-only in place:

| Corpus | Content | Scale | Property |
|---|---|---|---|
| `congress` | Congressional text data | 1.32M files, 14 GB | 8.6x compressible; extreme file count |
| `manga` | `.cbz` archives | 117 files, 27 GB | Genuinely incompressible; large files |
| `cb7` | Rust/Tauri project with a real build tree | 205k files, 42 GB | Mixed sizes; ~4 GB duplicated build artifacts |
| `docker-raw` | Live VM disk image | 130 GB allocated / 3.7 TB apparent | 28.6x sparse, 17,145 extents |

The harness refuses to use a corpus root as a destination.

### The independent oracle

`xsync-bench manifest` and `xsync-bench verify` pin native path-component bytes, object
kind, logical length, BLAKE3 content, permission and special mode bits, nanosecond mtime,
and raw symlink target, for the root and every descendant. Symlinks are never followed.
`verify` exits non-zero for any missing or unexpected path or any difference in content,
type, mode, mtime, or symlink target.

### Measurement discipline

- `xsync-bench schedule` produces a **rotated method order** so a candidate is never always
  measured after its baseline; the report builder independently rejects paired inputs whose
  ordering never crosses over.
- Every report records source revision, build profile, hardware, OS and kernel, destination
  filesystem, transport route, corpus manifest digest, tool versions, stream count,
  compression policy, and every individual sample — not just aggregates.
- Cache state must be labelled honestly. The allowed labels are `first_pass` ("first
  observation, no claim the kernel cache was empty"), `warm`, and `cold_evicted` — and
  `cold_evicted` is only valid alongside a non-empty description of the real eviction
  action taken. Calling a first repetition a cold-cache run is prohibited.
- `xsync-bench gate` compares against a baseline only when report schema, environment,
  session configuration and content-pinned corpus all match. **Correctness failures always
  fail, regardless of performance.** A paired speedup may degrade by at most 15%. Absolute
  wall time is never used as a historical gate. `--strict` fails on zero comparisons, so a
  missing baseline cannot produce an empty green check.
- Scratch directories are marker-owned; cleanup accepts exactly one direct child of the
  expected canonical base and refuses the base itself, the filesystem root, `$HOME`, the
  repository, nested or escaped paths, and tampered or missing markers.

Full usage: [`benches/README.md`](benches/README.md).

---

## 13. Operating it: state on disk, cleanup, troubleshooting

### What xsync leaves behind

Nothing removes these automatically.

| What | Where |
|---|---|
| BLAKE3 hash cache | `$XDG_CACHE_HOME/xsync/hashes.redb`, else `~/.cache/xsync/hashes.redb` |
| Resume journals | `$TMPDIR/xsync-resume-<16-hex>` |
| Staging files | `.xsync.tmp.<hash>` beside the destination file |

A leftover `.xsync.tmp.*` file is the signature of an interrupted transfer. Re-running the
same command is the correct response: it is safe, and the resume journal will skip verified
ranges of any large file.

### The single most common failure: remote PATH

xsync invokes the far end as `ssh HOST 'xs --server <path>'`. That is a **non-interactive**
SSH session, which on most Linux distributions does not source `~/.bashrc` or `~/.profile`,
so `~/.local/bin` may not be on `PATH`. xsync prefixes the remote command with
`PATH="$HOME/.local/bin:$PATH"` to handle the common case. Verify it directly on every
host before trusting anything else:

```bash
ssh freya 'xs --version'
```

If that fails but `ssh freya '~/.local/bin/xs --version'` succeeds, stage to a directory
already on the default non-interactive PATH, such as `/usr/local/bin/xs`.

### Expected noise

Every remote run prints `[xsync server] ...` diagnostic lines. Those are the *remote*
process's stderr, drained and echoed locally. `-q` silences local stdout but not these. For
cron:

```bash
xs -q ~/Documents/notes/ freya:/srv/backup/notes/ 2>/dev/null
```

### Version strings cannot distinguish builds

`xs --version` reports `0.1.0` on every host, and the workspace version was never bumped
even though a commit is titled "prepare v0.1.1". Peer compatibility is checked on
`PROTOCOL_VERSION`, not on the semver, so mixed builds interoperate as long as the protocol
version matches — but you cannot tell two builds apart. **Stage every host from the same
commit at the same time.**

### A round-trip verification you can run

```bash
mkdir -p /tmp/xs-check/src && head -c 5000000 /dev/urandom > /tmp/xs-check/src/probe.bin
```

```bash
xs /tmp/xs-check/src/ freya:/tmp/xs-check/pushed/ && xs freya:/tmp/xs-check/pushed/ /tmp/xs-check/back/ && shasum -a 256 /tmp/xs-check/src/probe.bin /tmp/xs-check/back/probe.bin
```

The two digests must match. That proves both directions and byte fidelity in one command.

---

## 14. Roadmap to rsync quality

### What "rsync quality" actually means

"As good as rsync" is not one goal, it is five, and they fail independently. A tool can be
twice as fast and still be unusable because it corrupts one file in ten thousand, or
flawless and still unused because nobody can install it.

| Axis | What it means | Where xsync is |
|---|---|---|
| **1. Correctness** | The destination is exactly right, every time, including after a crash, a full disk, a permission error, a hostile peer, or a file changing mid-read. | Strong foundations, one known integrity gap (§10), untested failure modes. |
| **2. Feature parity** | The flags people actually use work, or fail loudly with a clear reason. | Roughly 25% of rsync's real-world surface. |
| **3. Performance** | At least as fast as rsync on the workloads people run, and never dramatically worse on any of them. | Wins on large files, no-ops, and compression; **loses ~2x on many-small-files locally**. |
| **4. Distribution** | `brew install`, `apt install`, a signed binary, a man page, a version string that means something. | Nothing. Build it yourself. |
| **5. Trust** | Thirty years of production use, a security contact, a stable spec, a compatibility promise. | Pre-release, two-owner protocol, no disclosure policy. |

The phases below are ordered by what blocks the next thing, not by ambition.

---

### Phase 0 — Fix what is currently wrong

*Nothing else matters while any of these is open. All are small.*

**0.1 — Close the content-verification tautology.** The receiver computes the expected hash
from the buffer it just received, so the check always passes (§10). Only the declared length
is genuinely verified end to end for files under 32 MiB. **The sender's real digest must
reach the receiver for small and medium payloads, and be compared there.** This needs a new
protocol field, which means a version bump, which means it should be batched with any other
v1→v2 sync-path change. Until then, `--paranoid` is the only real content guarantee on that
path. *This is the single most important item in this document.*

**0.2 — The fork-per-file paths.** Two hot paths shell out to external binaries rather than
making a syscall, because the workspace denies `unsafe_code` and neither operation has a
safe wrapper in the dependency tree:

- `cloud::is_placeholder` runs `/usr/bin/xattr -p com.apple.fileprovider.fpfs#P <path>` —
  a **fork + exec per regular file**. It ran on macOS for *every* source file during
  planning, including under the default `--cloud-files=download` policy where the answer
  cannot change what happens. **Fixed:** detection is now gated on the `skip` and `error`
  policies, which are the ones whose outcome depends on it.
- `clone::platform_clone_file` runs `/bin/cp -c -p` (macOS) or `cp --reflink=always`
  (Linux) per cloned file, and `platform_clone_directory` runs `cp -a` per cloned tree.
  The 12 MiB clone threshold limits how often the per-file version fires, but it still
  forks for every large file. **Still open.**

The placeholder gate was measured back-to-back on `congress-10k`: median CPU **29.4 s →
7.0 s** and median wall **39.6 s → 8.2 s**, with the paired ratio against `rsync -a`
improving from 0.128 to 0.867 and all twenty oracle verifications passing. That is ≈1.98 ms
of CPU per file, the right order for `fork` + `exec` + `dyld` on macOS.

Two caveats that matter. The host was at **load average 265** for both runs, so the *before*
row is `noisy` under the project's own policy and the pair is **not gate-able** — a rerun on
an idle machine is still owed. And the direction is trustworthy only because the same-run
`rsync -a` baseline moved *against* the change (5.29 s → 7.34 s), so contention cannot
explain a 4.2x CPU reduction.

**A separate finding this turned up:** `cloud.rs` was added in `8ca26cce`, four commits
after the `f5e10179` revision stamped on every checked-in T1 report. No existing baseline
contains this cost — including the 0.515 paired ratio Story T1.3 records as the current
state (a `congress-10k` figure; see the scale note in §"Performance") — and TUNING.md §3's
"1.9 ms of kernel time per file" predates the code entirely, so its resemblance to the
number above is coincidence, not attribution. Evidence:
[`benches/results/tuning/T1/cloud-detection-gate/`](benches/results/tuning/T1/cloud-detection-gate/README.md).

For the clone path, the remedy is a vetted `reflink`/`clonefile` crate or a
narrowly-scoped `unsafe` block behind a documented exception.

**0.3 — Make Windows work as a server.** Detect a Windows peer and emit a `cmd.exe`-
compatible command line, and teach `is_missing_xsync_stderr` the cmd.exe wording
(`'xs' is not recognized as an internal or external command`) so a missing remote binary
produces a clean fallback rather than a raw transport error.

**0.4 — Bump the version and stamp the commit.** Four hosts all reporting `xs 0.1.0` is a
debugging trap. Embed the git revision and build profile in `--version`.

**0.5 — Fix the repository URL.** `Cargo.toml` still says `https://example.invalid/xsync`.

**0.6 — Repository hygiene.** `benches/results/tuning/` holds **22,654 files and 715 MB,
and they are committed**, not merely untracked as `DEPLOYMENT.md` records. That is now in
git history, so every clone pays for it forever and removing it requires a history rewrite.
Decide deliberately: keep the evidence and accept the size, or move raw reports to a
release-asset or separate-repository home and keep only the `DECISION.md` write-ups in
tree. This gets harder the longer it waits.

---

### Phase 1 — Correctness and integrity parity

*rsync's reputation is built on never being wrong. This is the phase that earns that.*

**1.1 — Failure-mode test coverage.** The tests prove the happy path and hostile *protocol*
input. What is barely tested is hostile *environment*: destination disk full mid-write,
permission denied on a subtree, a read-only destination, a killed remote process, a network
partition mid-chunk, a receiver crash leaving a partial journal, a source file deleted
between scan and read, a destination path that becomes a symlink between plan and write.
Each needs a test that asserts the destination is left in a defined state and the exit code
is right.

**1.2 — Fuzzing in CI.** There is a `fuzz/` directory with a protocol target. It is not run
by CI. A protocol whose entire safety argument is "we check every length before allocating"
should be continuously fuzzed, and the v2 message set needs its own target.

**1.3 — Supply-chain gates.** `cargo audit` and `cargo deny` in CI. The workspace denies
`unsafe_code`, but its dependencies do not, and that should be stated rather than implied.

**1.4 — Interrupted-run recovery, proven.** The `interrupted-resume` workload state exists
in the harness. Turn it into an assertion: kill a transfer at a known point, restart, and
prove via the oracle that the destination is byte-identical and that the resume journal
saved the bytes it claimed to save.

**1.5 — Cross-platform metadata semantics.** macOS stores symlink permission bits and Linux
forces 0777, which currently makes the `mixed` corpus unverifiable across a macOS→Linux SSH
route — `rsync -a` fails the identical check, so this is a platform limit rather than a
defect, but it needs a documented rule rather than an unrunnable test cell.

---

### Phase 2 — Feature parity

This is the largest body of work and the one most visible to users. The table below is the
rsync surface that matters in practice, with an honest status for each.

#### 2a. Filtering and selection — *highest user-visible impact*

| rsync | Status | Notes |
|---|---|---|
| `--exclude` | **Partial** | Globs only. Not rsync's filter-rule grammar. |
| `--include` | Missing | Needed for the extremely common "exclude everything except X" pattern. |
| `--exclude-from`, `--include-from` | Missing | Patterns from a file. |
| `--filter` / `-f`, merge files, `.rsync-filter` | Missing | The full rule language: `+`/`-` modifiers, anchoring, per-directory merge files, precedence. This is what makes rsync's filtering genuinely expressive, and it is a substantial parser plus a well-specified matching order. |
| `--files-from`, `-0` | Missing | Explicit file list on stdin. Very common in scripted backups. |
| `--prune-empty-dirs` / `-m` | Missing | |
| `--max-size`, `--min-size` | Missing | Cheap to add, frequently used. |
| `--existing`, `--ignore-existing` | Missing | Cheap, common. |
| `--update` / `-u` | Missing | Skip files newer at the destination. Cheap, very common. |
| `--size-only`, `--ignore-times` | Missing | Both are one-line changes to the classifier and both are used constantly in the field. |
| `--relative` / `-R` | Missing | Changes the destination path shape. |

**Recommendation:** do the cheap classifier flags (`-u`, `--size-only`, `--ignore-times`,
`--max-size`, `--min-size`, `--existing`, `--ignore-existing`) as one small batch first —
they are individually trivial, they cover a large fraction of real invocations, and they
need no protocol change. Then treat `--filter` as its own project with its own spec
document and conformance tests, because a filter language that is *almost* rsync's is worse
than one that is obviously different.

#### 2b. Metadata preservation

| rsync | Status | Notes |
|---|---|---|
| `-t` times, `-p` perms, `-l` links, `-r` recursive | **Done** | Always on; there is no way to turn them off. |
| `-o` owner, `-g` group, `--numeric-ids`, `--chown` | Missing | Needs privilege handling and a uid/gid mapping policy. Required for any real system-backup use. |
| `-H` hardlinks | Missing | Needs an inode→path map across the whole run and a protocol representation. Genuinely hard, and memory-bounded design is the interesting part. |
| `-A` ACLs, `-X` xattrs | Missing | Platform-specific; also the natural home for macOS resource forks and Finder metadata. |
| `-D` devices and specials | Missing | Blocks system-level backup entirely. |
| `-S` sparse | Missing | See Phase 3; currently a data-loss-adjacent hazard, not just a missing feature. |
| `--chmod` | Missing | |
| `-a` archive | N/A | xsync is always archive-ish. Once `-o`/`-g`/`-D` exist, an explicit `-a` and per-attribute opt-outs become necessary. |

#### 2c. Transfer behaviour

| rsync | Status | Notes |
|---|---|---|
| Whole-file transfer (`-W`) | **Done** | This is xsync's only mode. |
| Delta transfer (the rsync algorithm) | Missing | The defining rsync feature. See Phase 3.4. |
| `--partial`, `--partial-dir` | **Superseded** | xsync's durable journal is stronger for large files, weaker for small ones (which restart). |
| `--inplace`, `--append`, `--append-verify` | Missing | Conflicts with atomic staging; needs an explicit opt-out of the atomicity guarantee. |
| `--link-dest`, `--copy-dest`, `--compare-dest` | Missing | These are how people build snapshot backups with rsync. Notably, xsync's clone fast path is a *better* primitive for the same job — this is an opportunity, not just a gap. |
| `--backup`, `--backup-dir`, `--suffix` | Missing | |
| `--temp-dir` | Missing | Staging location is currently always beside the destination. |
| `--bwlimit` | Missing | Genuinely important on shared links; also the thing that makes a bandwidth-limited benchmark route meaningful. |
| `--timeout`, `--contimeout` | Missing | A hung transfer currently hangs forever. |
| `--write-batch`, `--read-batch` | Missing | Niche. |
| `--fuzzy` | Missing | Niche. |
| `--compress-choice`, `--skip-compress` | **Superseded** | Sampling makes `--skip-compress` unnecessary and is measurably better than an extension list. Worth documenting as a deliberate divergence. |

#### 2d. Deletion

| rsync | Status |
|---|---|
| `--delete` | **Done** (after-transfer semantics) |
| `--delete-before`, `--delete-during`, `--delete-delay`, `--delete-after` | Missing — xsync always does delete-after, which is the safest and is a defensible permanent choice |
| `--delete-excluded` | Missing |
| `--max-delete` | Missing — **this is a safety feature**, and it is the one that stops a misconfigured mirror from wiping a destination |
| `--force` | Missing |

**Recommendation:** `--max-delete` should be prioritized above the ordering variants. It
prevents a class of user-caused disaster that no amount of correctness work addresses.

#### 2e. Output and observability

| rsync | Status | Notes |
|---|---|---|
| `-v`, `-q` | **Partial** | `-q` exists; there are no verbosity levels. |
| `--progress` / `-P` | **Partial** | A two-bar terminal UI exists. `-P` (progress + partial) has no equivalent. |
| `--stats` | **Partial** | The final summary covers it; no dedicated flag. |
| `--itemize-changes` / `-i` | Missing | The `YXcstpoguax` change string. Heavily used in scripts. The `action` event already carries the information. |
| `--out-format`, `--log-file`, `--log-file-format` | Missing | |
| `--list-only` | Missing | Now trivially implementable on top of the v2 `List` message. |
| `--dry-run` / `-n` | **Done** | |
| JSONL event stream | **Better than rsync** | rsync has nothing comparable. Keep it, version it, and document it as a first-class interface. |

#### 2f. Transport and topology

| rsync | Status | Notes |
|---|---|---|
| SSH transport | **Done** | |
| `-e`/`--rsh` | **Done** | |
| Remote → local, local → remote | **Done** | |
| Remote → remote | Missing | Rejected at parse. Documented workaround exists. |
| rsync daemon (`rsync://`, `rsyncd.conf`, modules, auth) | Missing | An entire subsystem: anonymous and authenticated modules, per-module paths, chroot, host allow/deny. Large surface, and questionable whether xsync should reimplement it versus offering a modern authenticated daemon (already planned for v2). |
| `--port`, `--address`, `--sockopts` | Missing | Daemon-only. |
| `--protocol` | Missing | |
| rsync wire compatibility | **Partial** | Push and pull, GNU protocol 32 only. openrsync 29 and OpenBSD 27 are research targets. |

---

### Phase 3 — Performance parity

*The one measured, reproducible loss is many-small-files on the local path. Everything here
serves closing that, plus the structural wins that only a different architecture can get.*

**3.1 — Attribute and close the per-file syscall gap** *(the prerequisite for every local
performance claim)*. Target: `congress-10k` initial copy with system time within 1.5x of
`rsync -a` and a wall-clock paired ratio of at least 0.9, over five repetitions with
MAD/median ≤ 15%, confirmed an order of magnitude up on `congress-100k`, with no regression
on large files or on the no-op case (currently 1.91x ahead).

Current status: **not met at `congress-10k`; met by a wide margin at `congress-100k`.**
The 0.515x figure is a `congress-10k` measurement. The gate's own confirmation step — "an
order of magnitude up on `congress-100k`" — now passes decisively: R0a measured a **4.34x**
paired ratio there (xsync 5.79 s, `rsync -a` 25.72 s, 2026-08-31). xsync's local time is
nearly flat in file count while rsync's is linear, so the gate is really asking about the
small end of the curve.

Two of three known waste items are fixed (read-buffer sizing, the 12 MiB clone threshold);
the third (temp-path hash caching) was measured, showed no useful gain, and is honestly
reported as such. **The 10k figure has not been re-measured since the clone work landed**,
so the first action on this story is a current `congress-10k` local run, not more
optimisation.

The attribution trace is **blocked**: macOS SIP produces empty `dtruss` tables even under a
user-run privileged capture, and `fs_usage` requires root. Options are a Linux host for the
histogram, a signed tracing helper, or `ktrace`/`Instruments`. **Do not proceed by
guessing** — but note that Phase 0.2's placeholder gate was found and measured by
inspection alone, without a tracer, and the same approach applies to the clone path.

**3.2 — Get the parallelism shape right.** Evidence suggests xsync parallelizes the wrong
half. Eight threads calling `renameat` on APFS moved 13k/s to 14k/s, because the filesystem
serializes directory metadata mutation — while parallel *copying* on the same machine hit
2.43x. xsync currently uses one uniform worker pool for both. The work is a 1-to-16 sweep
on `congress-10k` and `congress-100k`, separately for the metadata phase and the data
phase, on both APFS and ext4, producing a documented policy: probably a small fixed
metadata concurrency with data concurrency scaled to device queue depth.

**3.3 — Clone at the highest unchanged subtree.** Today the whole-tree clone only fires on
a completely fresh copy; an incremental sync falls back to per-file cloning, which is 2.70x
where tree-level cloning of identical bytes is 22x. Identify maximal subtrees that are
wholly unchanged or wholly absent and clone at that root. Success: `congress-100k` with one
changed subtree completes in time proportional to the *changed subtree*, not the tree.

**3.4 — Decide on delta transfer with a measurement, not an assumption.** Before
implementing content-defined chunking, measure whether the win exists: run FastCDC over the
`cb7` build tree, build a chunk index, and compute unique versus total chunk bytes across
two consecutive builds. The bar is a unique-byte fraction below 70% on a first sync and
below 20% across two builds. Below that, the complexity is not justified.

Note the shape of the opportunity: a content-addressed destination chunk index makes
renames, copies and cross-file duplication free — something **rsync structurally cannot
do**, because its delta only ever compares a file against the same path. `cb7` holds two
byte-identical 165 MB `.rlib` files at different paths; rsync sends both.

**3.5 — Sparse-aware transfer.** Currently deferred, and it is the difference between
"cannot complete" and "completes" on VM images. Enumerate allocated extents with
`SEEK_HOLE`/`SEEK_DATA` (portable across APFS, ext4, btrfs, XFS; Windows needs
`FSCTL_QUERY_ALLOCATED_RANGES`), transfer only data extents, and reproduce holes by seeking
rather than writing zeros. The extent distribution is the real test — sizes span 4 KiB to
5.62 GB, so the enumerator must handle a single 5.6 GB extent and thousands of single-block
extents without degenerating into per-block I/O. Detection must degrade safely: a
filesystem that does not report holes falls back to dense transfer with a recorded warning,
**never** to silent truncation. Baseline is `rsync -aS`. This also requires adding
*allocated bytes* as a fourth reported quantity alongside logical, physical and wire.

**3.6 — Measure the regimes nobody has measured.** Every benchmark route so far is a LAN or
a local pipe with a warm cache — precisely the regime where compression and dedup do not
matter. Compression delivered a 713x wire reduction that produced no wall-time advantage
because the link was never the constraint. Add a bandwidth-limited route (traffic shaping
at 50 / 100 / 1000 Mbit with injected latency) and a genuine cold-cache mode, then publish
the crossover table: the link speed below which compression and dedup dominate, and above
which syscall cost dominates. **Every future performance claim should cite which regime it
belongs to.**

**3.7 — Run the tiers that have never been run.** Everything published is `smoke`. Run the
regression (100k-entry) tier, nominate a checked-in gate baseline, and wire the gate into
CI so a performance regression fails a build instead of being discovered a month later.

---

### Phase 4 — Distribution and trust

*rsync's real advantage is that it is already on the machine. This phase is what turns
"a binary I built" into "a tool I can install."*

**4.1 — Release artifacts.** CI already builds every Tier 1 target and uploads none.
A tarball per target plus `SHA256SUMS` would replace manual staging entirely and, notably,
would give a Windows machine a binary without requiring a Visual Studio install.

**4.2 — Signing.** macOS signing and notarization; Windows Authenticode. Without these, a
downloaded binary is blocked by Gatekeeper and flagged by SmartScreen — which in practice
means nobody outside the author runs it.

**4.3 — Packaging.** An install script, a Homebrew formula, Linux packages (`.deb`/`.rpm`),
a Windows distribution path, and a crates.io publish.

**4.4 — Getting xsync onto the far end.** This is the difference between "install xsync on
both machines" and "run xsync", and it matters more than it looks. Three parts: diagnose
clearly when the remote binary is missing (partly done — there is a clean
`MissingRemoteXsync` error and an rsync fallback); an *explicitly authorized*, separately
visible remote bootstrap (never implicit, and never triggered merely by
`--transport=auto`); and a written version/protocol compatibility policy.

**4.5 — The install experience.** Shell completions, a man page, first-run documentation,
and an uninstall path that cleans up the hash cache, journals and staging files listed in
§13.

**4.6 — Scheduling.** Nothing here is a daemon; every transfer is a one-shot command. A
systemd timer per Linux host and a launchd agent on macOS is the smallest thing that turns
this from "a tool I run" into "a network that stays in sync." A Windows service and tray
follow.

**4.7 — Security posture.** A written statement of the remote-server trust model,
destination path containment, symlink handling, protocol allocation limits, and temp and
journal cleanup — plus a security contact and disclosure policy, **before** the first
public release, not after the first report.

**4.8 — A support matrix.** Published platforms, tiers, glibc floor, and known limitations,
with the sparse-file hazard called out prominently until Phase 3.5 lands.

---

### Phase 5 — Protocol and ecosystem

**5.1 — Spec ownership.** The wire format now has two consumers. Write down who may change
`protocol.md`, what review the other project gets, and how a change lands when both projects
need it simultaneously (neither can merge a half-implemented wire format). The type-byte
rule — assigned once, never reused, never reinterpreted — has to survive two owners.

**5.2 — A generated compatibility matrix.** Which client versions work with which server
versions, where every cell is one of *works*, *works with reduced capability (naming
which)*, or *refuses with a specific message*. **No cell may be "undefined."** It should be
generated from the conformance vectors rather than maintained by hand. It must cover the
asymmetric case both projects will actually hit: a long-lived agent staged months before
the client that connects to it.

**5.3 — A joint smoke test.** One command that runs a real second-party client against a
real xsync server — connect, list, stat, fetch, publish, mutate, disconnect — before either
project tags a release. Unit tests on either side catch disagreements about *bytes*; only
this catches disagreements about *meaning*. Its failure output must identify which side is
wrong, which is the entire reason it exists.

**5.4 — Expand the conformance vectors.** Byte-exact vectors exist for the v2 message
table. Extend them to cover the v1 sync messages, so a second implementation of the *sync*
path is possible and not just the browse path.

---

### The v2 and v3 horizon

Beyond parity, these are the things that would make xsync structurally better rather than
incrementally faster:

- **A persistent index and change journal.** rsync's real structural weakness is that it
  rebuilds the entire file list on every run: O(tree) work regardless of how much changed. A
  daemon holding a live index turns re-sync into O(changes), which is a *categorical* win
  rather than a constant-factor one. A warm index was measured at 25.2 ms against 300.3 ms
  for `readdir` + `fstatat` — 11.9x. The target is producing a plan for a 1.32M-entry tree
  with <1% changed in under one second, against a multi-minute full walk.

  **The correctness trap is the whole story here.** Filesystem event streams are *hints*,
  not truth. FSEvents raises `MustScanSubDirs` / `UserDropped` under a 40,000-file burst,
  and a client that ignores those flags goes permanently and silently stale. A dropped
  subtree must trigger a rescan, and that must be tested explicitly. An index is only worth
  having when its invalidation contract is correct.

- **Delta transfer**, gated by the cost model in Phase 3.4 rather than assumed.
- **Remote → remote**, or a documented, supported pattern for it.
- **A native authenticated transport** and daemon, with services and a tray UI.
- **Platform-specific I/O** — `io_uring`, no-cache hints — only after isolated benchmarks
  prove a benefit with no correctness or cache-pressure regression.

Explicitly deferred to v3: a real home directory as a benchmark corpus (pathological
metadata at scale), and APFS-compressed file handling.

---

### Sequencing, in one table

| # | Work | Blocks | Effort |
|---:|---|---|---|
| 1 | Content-verification gap (0.1) | Any honest correctness claim | S, but needs a protocol bump |
| 2 | Fork-per-file investigation (0.2) | All local performance work | S |
| 3 | Version stamping, repo URL, hygiene (0.4–0.6) | Any release | S |
| 4 | Cheap classifier flags — `-u`, `--size-only`, `--max-size`, … (2a) | Real-world usability | S |
| 5 | `--max-delete` (2d) | User safety | S |
| 6 | Failure-mode tests + fuzzing in CI (1.1, 1.2) | Trust | M |
| 7 | Syscall gap + parallelism shape (3.1, 3.2) | The one measured loss | M–L |
| 8 | Release artifacts + signing (4.1, 4.2) | Anyone else using it | M |
| 9 | Windows as a server (0.3) | Windows parity | M |
| 10 | Subtree clone (3.3) | Incremental-sync performance | M |
| 11 | `--itemize-changes`, `--list-only`, verbosity (2e) | Scriptability | M |
| 12 | Sparse support (3.5) | VM-image workloads (currently impossible) | M |
| 13 | Ownership, devices, `--numeric-ids` (2b) | System backup | M |
| 14 | Filter-rule language (2a) | Filtering parity | L |
| 15 | Packaging, scheduling, docs (4.3–4.8) | Adoption | L |
| 16 | Hardlinks, ACLs, xattrs (2b) | Full archive parity | L |
| 17 | Delta transfer / CDC (3.4) | WAN parity | L, gated on measurement |
| 18 | Persistent index + daemon (14.8) | Categorical win over rsync | XL |

### When is it done?

A defensible claim of "rsync quality" needs all of:

- [ ] No paired benchmark cell below **0.9x** of its rsync baseline on any supported route,
      at regression tier, with the correctness oracle passing on every repetition.
- [ ] Every rsync flag in §14 Phase 2 either implemented or **rejected before mutation with
      a specific, actionable message**. No flag silently ignored, and no flag silently doing
      something subtly different.
- [ ] Real end-to-end content verification on by default — not only under `--paranoid`.
- [ ] Every failure mode in Phase 1.1 covered by a test that asserts destination state and
      exit code.
- [ ] Continuous fuzzing and supply-chain auditing in CI.
- [ ] A signed, packaged binary installable from a package manager on all Tier 1 targets.
- [ ] A published support matrix, security policy, and disclosure contact.
- [ ] A generated compatibility matrix with no undefined cells.

---

## 15. Repository layout

```
crates/
  xsync/            The `xs` binary: clap CLI, terminal progress UI, JSONL renderer
    src/main.rs     Argument surface, transport routing, event rendering
    build.rs        Packages and validates the Windows server launcher
    resources/      xsync-server.cmd
    tests/          23 end-to-end server integration tests
  xsync-core/       The engine library
    scanner.rs      Parallel walk, bounded channel, source fingerprints
    path.rs         rsync-style path specs; raw-byte wire paths; hostile-path rejection
    planner.rs      Destination index (memory-bounded, disk-spilling), classification
    strategy.rs     Size bucketing, batch coalescing, bounded work queues
    source.rs       Stable source reads that cannot bless two file versions
    sink.rs         Deterministic staging, verification, atomic publication
    clone.rs        APFS clonefile / Linux reflink fast paths
    local.rs        In-process local→local pipeline and the event enum
    server.rs       Remote server, SSH/pipe clients, multi-stream, v2 browse session
    protocol.rs     v1 framing, message codec, capability negotiation
    protocol_v2.rs  v2 browse/mutate/transfer message codec
    journal.rs      Durable resume journal, range merging, chunk accounting
    hash_cache.rs   redb-backed BLAKE3 cache with batched commits
    compression.rs  Bounded compression sampling
    cloud.rs        macOS cloud-placeholder detection
    transport.rs    Transport identity and capability reporting
benches/            xsync-bench: corpora, oracle, reports, gates (no xsync-core dependency)
  engine/           Standalone spikes: scanner, clone, stripe, connection, remote
  results/          Checked-in evidence and DECISION.md per story
  scripts/          release-bench.py, remote-matrix.py, release-matrix.py
fuzz/               cargo-fuzz protocol target (not yet in CI)
protocol-v2-vectors/  Byte-exact cross-project conformance vectors
build/              Dockerfiles pinning the Linux glibc 2.28 floor and the musl toolchain
scripts/            stage-linux.sh, deploy-mars.sh
docs/               Target matrix, JSONL schema, rsync wire contract, browse model
```

---

## 16. Document index

| Document | What it is |
|---|---|
| [`plan.md`](plan.md) | The v1 design: thesis, scope, semantics, pipeline, protocol, benchmark policy, implementation order |
| [`tasks.md`](tasks.md) | v1 epics, stories and acceptance criteria with status, plus implementation reports |
| [`protocol.md`](protocol.md) | The wire format contract — envelope, encoding, both message tables, bounds |
| [`v2handshake.md`](v2handshake.md) | Version negotiation and graceful degrade, with its required test matrix |
| [`backlogv2.md`](backlogv2.md) | v2 epics: protocol v2, browse, mutation, single-file transfer, two-owner governance |
| [`TUNING.md`](TUNING.md) | Why synthetic corpora were retired, the four real corpora, the measured baseline, spikes S1–S8 |
| [`TUNING-TASKS.md`](TUNING-TASKS.md) | Executable work breakdown for the tuning epics, with blockers recorded |
| [`DEPLOYMENT.md`](DEPLOYMENT.md) | Build, CI, artifacts, signing, packaging, services, install experience |
| [`MVP.md`](MVP.md) | Operational guide for running `xs` across a four-host home network *(defect list is stale — see §2)* |
| [`benches/README.md`](benches/README.md) | Full harness usage: corpora, oracle, scheduling, reports, gates, scratch safety |
| [`docs/TARGET-MATRIX.md`](docs/TARGET-MATRIX.md) | Tier 1 / Tier 2 / not-built targets and ABI baselines |
| [`docs/progress-json-v1.md`](docs/progress-json-v1.md) | The `--progress-json` event schema |
| [`docs/rsync-wire-v1.md`](docs/rsync-wire-v1.md) | The rsync-protocol fallback's implemented subset and its provenance |
| [`docs/browse-connection-model.md`](docs/browse-connection-model.md) | Connection lifetime, reconnect obligations, transfer isolation |
| [`docs/linux-staging.md`](docs/linux-staging.md) | The staging script's contract |
| [`benches/results/story-8.1/DECISION.md`](benches/results/story-8.1/DECISION.md) | The release benchmark matrix: findings, fixes, open defects, decision |

---

## License

MIT OR Apache-2.0.
