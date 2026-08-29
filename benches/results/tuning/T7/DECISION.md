# T7 — parallelism shape: a real platform split, and a corrected default

Date: 2026-08-24 (invalid attempt), redone 2026-08-28.

## Correction carried forward

An earlier revision of this file reported directory-affine dispatch as measured
and rejected. **That experiment was void** — every run executed
`target/release/xsync`, a stale orphan left when the binary was renamed to `xs`,
so both arms ran identical four-day-old code. The harness now defaults to
`target/release/xs` and `assert_binary_is_current()` refuses to benchmark a binary
older than the sources. The affinity hypothesis remains **untested**, and is now
also largely moot — see below.

## What changed the question

T7 was originally motivated by a profile showing workers spending **18.58%** of
runtime in crossbeam `recv<FileTask>`, contending on the shared task queue. After
T1.7 removed the doomed reflink attempts that were stalling those workers, a fresh
profile puts the same frame at **2.04%**. The contention was largely a symptom of
the clone path, not an independent problem. Dispatch partitioning is therefore no
longer the interesting question; worker *count* is.

## Worker sweep

`--local-workers` was added to the CLI for this (T7.1 recorded its absence as the
blocker). Sweeps use `--no-directory-clone` on APFS so per-file work actually
happens; without it congress is published as a single directory clone and the
worker count is irrelevant.

**Linux, ext4/NVMe, 24 cores, congress-10k, 5 repetitions (MAD 0.1–0.9%):**

| workers | 1 | 2 | 4 | 6 | 8 | 12 | 16 | 24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| median | 0.468 s | 0.317 s | 0.241 s | 0.221 s | 0.209 s | 0.201 s | 0.196 s | **0.193 s** |

Monotonic, 2.43x from 1 to 24, no degradation at the top. 8 workers already
capture 92% of the available gain.

**macOS, APFS, 10 cores, congress-10k, 9 repetitions:**

| workers | 2 | 3 | 4 | 5 | 6 | 10 (default) |
|---|---:|---:|---:|---:|---:|---:|
| median | 2.856 s | 2.609 s | 2.520 s | **2.500 s** | 2.554 s | 2.717 s |

**macOS, APFS, cb7 `node_modules` (26,605 files, 7.2 per directory), 5 repetitions:**

| workers | 2 | 4 | 5 | 8 | 10 (default) | 16 |
|---|---:|---:|---:|---:|---:|---:|
| median | 4.893 s | **3.834 s** | 3.872 s | 4.534 s | 4.418 s | 4.474 s |

## Finding

**The two platforms differ in kind, not degree.** ext4 scales monotonically to the
core count. APFS peaks at 4–5 workers and then *degrades*, and this replicates
across two corpus shapes that share almost nothing — congress has one file per
directory, `node_modules` has 7.2 and a 3,920-file outlier. This is f2 §2's
observation (eight `renameat` threads moving 13k/s to 14k/s because APFS
serializes directory metadata mutation) reproduced inside xsync.

The shipped default was one worker per logical core on both platforms, which on
this Mac meant 10 — **8% worse than optimal on congress and 15% worse on
`node_modules`.**

## Change

`default_local_workers()` now caps at 4 on macOS and remains one-per-core
elsewhere, with the measurements recorded at the definition. `--local-workers`
overrides it for measurement or unusual workloads.

Verified no regression on the case that might have suffered from fewer workers:
four large `.cbz` files copy in 0.520 s at both 4 and 10 workers, since large-file
work is not metadata-bound.

## Honest limits

- The cap is a constant chosen from two corpora on one Mac. It is right in
  direction and roughly right in magnitude; it is not a tuned value for every
  Apple machine, and a device with very different core counts may want a
  different ceiling.
- Only APFS and ext4 were measured. Windows, ZFS, btrfs, and XFS are unknown, and
  the `cfg!(target_os = "macos")` test is a proxy for "the filesystem serializes
  metadata", which is not the same statement.
- Directory-affine dispatch remains untested. It is now a much smaller prize
  (2.04% rather than 18.58%), so it should stay closed unless a profile puts
  `recv<FileTask>` back near the top.
