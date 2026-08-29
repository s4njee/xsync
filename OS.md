# Operating system comparison

What changes when the same file-sync engine, the same corpora and the same
protocol run on macOS, Linux and Windows. Measurements are drawn from
`BENCHMARKv2.md`; this document is the cross-OS reading of them.

Every figure here is a median with MAD reported at source. Where a host was in
normal desktop use rather than dedicated, scaling *shapes* are treated as sound
and absolute cross-machine figures are not.

## The hosts

| Host | OS | CPU | RAM | Notes |
|---|---|---|---:|---|
| MacBook | macOS 26.6.1 | M1 Max, 10 cores (8P+2E) | 64 GB | active workstation |
| freya | Linux (CachyOS 7.1.1) | Ryzen 9 7950X, 32 threads | 61 GB | k3s stopped for benchmarking |
| orion | Linux (Gentoo 6.18) | Raspberry Pi 5, 4 cores | 3 GB | dedicated |
| 7900x | Windows 11 Pro 26200 | Ryzen 9 7900X, 24 threads | 31 GB | **dual-boots as `mars`** |

The last row is what makes this document possible: the Windows host and the Arch
Linux host `mars` are **the same physical machine**. Comparisons between them
differ by operating system, not hardware.

## The headline: Windows is 5.7x slower over the network

Identical corpus, identical source Mac, identical gigabit link, identical
machine at the far end — only the booted OS differs.

| Target OS on the 7900X | congress-100k | Files/s |
|---|---:|---:|
| Arch Linux (`mars`) | 17.3 s | 6,336 |
| Windows 11 | 99.8 s | 1,099 |

**5.7x.** Caveat, stated because it is not fully controlled: the two operating
systems live on different NVMe drives (Kingston for Windows, SPCC for Arch). The
CPU, memory, board and network are identical.

## Local copy, same corpus, three operating systems

congress-100k, local disk to local disk, each host at its own best worker count.

| Host | OS | Time | Files/s | Conditions |
|---|---|---:|---:|---|
| freya | Linux | 2.2 s | 49,800 | warm cache, ZFS -> ext4 |
| 7900x | Windows | 47.8 s | 2,294 | warm cache, NTFS -> NTFS, Defender on |

The Windows figure is **21.7x** the Linux one on comparable-class hardware. Both
are warm-cache; neither is bandwidth-bound.

## Against each platform's native tools

xsync versus what ships with the OS, same corpus, same conditions.

| OS | Tool | Time | Ratio to xsync |
|---|---|---:|---:|
| Windows | `robocopy /MT:16` | 27.5 s | **1.74x faster** |
| Windows | **xsync** (16 workers) | 47.8 s | — |
| Windows | `xcopy` | 60.1 s | 0.79x |
| macOS | **xsync** (16 workers) | 1.95 s | — |
| macOS | `tar c \| tar x` | 2.01 s | 0.97x |
| macOS | `rsync -a` | 3.43 s | 0.57x |
| macOS | `cp -a` | 5.89 s | 0.33x |

xsync leads every native tool on macOS and loses to `robocopy` on Windows.

### robocopy's number comes with conditions

On a corpus containing symlinks, `robocopy` either **hangs** — its defaults are
`/R:1000000 /W:30`, a million retries thirty seconds apart — or, with `/XJ`,
**silently skips them**: verified at 0 symlinks created against a source with
3,310. xsync recreated all 3,310. The cb7 comparison was withdrawn for this
reason; congress (no symlinks) remains the fair one.

## Worker scaling behaves differently per OS

Normalised to each host's own single-worker time.

| Host | OS | Cores | Best worker count | Past the optimum |
|---|---|---:|---:|---|
| freya | Linux | 32 | 32 | flat to 64 |
| orion | Linux | 4 | 16-32 | flat past 16 |
| MacBook | macOS | 10 | 8 | **degrades**, -6.5% at 32 |
| 7900x | Windows | 24 | 16 | mild decline at 24 |

**Linux tolerates over-provisioning; macOS punishes it.** That difference is what
`MACOS_WORKER_CAP` exists for, and it was confirmed rather than assumed — the
degradation survived a re-run on a cooled drive in descending order.

No single variable predicts the optimum. An 8x range of core counts (4 to 32)
moved it hardly at all, and two hosts writing to *the same physical SSD* reached
different optima.

## `--streams` is actively harmful on Windows

Multi-stream transfers over SSH to each OS, congress-100k.

| Streams | To Windows | To Linux (cb7, after fix) |
|---|---:|---|
| 1 (default) | **99.8 s** | baseline |
| 2 | 143.5 s | — |
| 4 | 152.4 s | — |
| 8 | 149.8 s | — |

Adding streams costs **1.44x to 1.53x** on Windows. Bracketed control confirms no
drift: opening arm 99.77 s, closing arm 98.51 s.

This is the opposite of Linux, where the same flag is a modest win on large-file
corpora. On Windows, N+1 SSH connections plus per-connection process spawn cost
more than the parallelism returns.

Until today `--streams` did not work against Windows at all — see the bug list.

## Cross-device separation helps robocopy, not xsync

Reads and writes on separate physical devices (Kingston NVMe -> Inland SATA):

| Tool | Same device | Cross device | Improvement |
|---|---:|---:|---:|
| xsync (16 workers) | 47.78 s | 47.94 s | **1.00x** |
| `robocopy /MT:16` | 27.47 s | 16.16 s | **1.70x** |

robocopy converts the removal of read/write contention directly into throughput.
xsync gains nothing, which says its bottleneck is **not disk contention** but
per-file work — consistent with 40-53% system time measured during these runs.

## Filesystem semantics that differ

| Behaviour | macOS (APFS) | Linux (ext4/ZFS) | Windows (NTFS) |
|---|---|---|---|
| Case sensitivity | insensitive | sensitive | insensitive |
| Hardlinks | detected + reported | detected + reported | **not detectable** on stable Rust |
| Extended attributes | detected + reported | detected + reported | **ADS not detectable** |
| Sparse files | detected with byte figures | detected with byte figures | detected, no byte figures |
| Symlinks | preserved | preserved | preserved (needs Developer Mode) |
| Junctions | n/a | n/a | converted to symlinks |
| Ownership | compared | compared | no Unix uid/gid; reported as unsupported |
| Reflink / clone | `clonefile` | `FICLONE` | none; probe declines cleanly |

Nothing is silently unpreserved on any platform: what cannot be detected is
named as unchecked in the run summary and in the `finished` event, so silence
never means "your source had none".

## Bugs that only Windows exposed

Each was invisible on macOS and Linux because `cfg(unix)` compiled the broken
path out, or because no Unix host exercised it.

| Bug | Consequence |
|---|---|
| `xattr` was an unconditional dependency | Windows build failed outright |
| `note_dropped_metadata` defined twice | Windows build failed; `cfg(unix)` hid it elsewhere |
| `clone_spike.rs` used an unbound `status` | Windows build failed |
| `--streams` never probed the remote shell | every multi-stream transfer to Windows died |
| `permission_mode` ignored file type | **pulls from Windows produced directories with no execute bit — data transferred correctly and was unreachable** |

The last one is the most serious and the least likely to have been found without
running the platform: the transfer reports success, verifies its hashes, and
leaves a tree nobody can enter.

## What would change these numbers

- **Windows Defender** is the leading hypothesis for much of the platform gap and
  is unmeasured. It was the top CPU consumer throughout at 4.5x the next process.
  Measuring the delta is backlog item 4.12 — deliberately *not* by recommending
  anyone disable it, since the default configuration is what users run.
- **Cold-cache measurement on Windows** is not yet possible: there is no
  `drop_caches` or `purge` equivalent. Every Windows figure here is warm, while
  the Linux and macOS cold figures exist. A corpus larger than RAM is the
  workaround.
- **The drives are not controlled** across operating systems. Four SSDs were
  characterised during this work and three of them misled a measurement.
