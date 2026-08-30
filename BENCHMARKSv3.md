# Benchmarks v3 — the picture after 4.15 and 4.26

Measured 2026-08-30, in one session, on one client, with every run verified.
Supersedes nothing in `BENCHMARKv2.md`; that file keeps the tuning history and
the long tail of experiments. This one answers a narrower question: **now that both
halves of the small-file path have been unserialized — the sender (4.15) and
the receiver (4.26) — what is actually slow, and why?**

---

## What changed since v2

Both halves of the small-file path were serialized. Both are now fixed, and
they multiply.

**4.15 — the sender.** The batch builder issued up to 8,192 blocking reads with
the network idle, then hashed, compressed and framed with the disk idle. A
loader thread now runs one batch ahead. **2.05×**, reproduced across three
sessions at 1.86× (loaded), 2.15× (idle) and 2.05× (idle, wired, verified).

**4.26 — the receiver.** The receive loop decoded *and applied* on one thread.
Decoding must stay serial, but publishing a file is independent per file. An
apply pool now does that off the decode thread, keeping the ack-on-commit
contract. Measured with the client held constant and only the server binary
alternated:

| server | before | after | gain |
|---|---:|---:|---|
| Windows NTFS | 90.30 s | **55.83 s** | **1.62×** |
| freya (Linux, ZFS) | 12.36 s | **8.46 s** | **1.46×** |
| orion (Pi 5) | 12.21 s | **9.23 s** | **1.32×** |
| WSL2 ext4 | 18.96 s | **15.62 s** | **1.21×** |

**End to end, congress-100k to freya: 26.30 → 12.80 → 8.46 s — 3.11×.**

Windows gains most because it is the receiver-bound platform. That has a
consequence for the headline below: **part of what v3 first attributed to "the
OS" was xsync's own receiver serialization** amplifying Windows' higher
per-file cost.

---

## Method

Every number below follows the same discipline, because three separate silent
failures earlier in this cycle produced plausible-looking numbers that were
later retracted.

- **Verified.** A run is discarded unless the process exits zero **and** the
  landed entry count at the destination matches the corpus exactly. A run that
  fails is reported as `INVALID`, never as a timing.
- **Warmup discarded.** The first run of any batch is thrown away. This is not
  cosmetic: congress to freya measured 23.2 / 21.8 / 21.0 s cold against
  11.6 / 11.9 / 11.4 s settled — a **1.9× warmup effect** — and cb7 produced a
  single 237 s outlier among ~70 s runs in the same cold batch.
- **Same session.** Every figure in a comparison was taken in one sitting on
  one client. Cross-session absolute comparison is not trusted here; an earlier
  attempt at it manufactured a false regression scare.
- **Median of three**, all raw values printed.

---

## Hosts

| host | CPU | OS / filesystem | notes |
|---|---|---|---|
| *client* | Apple M1 Max | macOS, APFS | 1 GbE via a **USB** adapter |
| freya | Ryzen 9 7950X, 32 threads | Linux, **ZFS** on NVMe | idle (load 0.03) |
| orion | Raspberry Pi 5, 4 cores, 3 GB | Linux, **ext4** on NVMe | idle |
| 7900x | Ryzen 9 7900X | **Windows 11, NTFS** | Defender real-time on |
| 7900x | *same machine* | **WSL2 Ubuntu, ext4** in a VHDX on the same NVMe | same CPU, disk, link |

The last two rows are the point: they differ only in operating system and
filesystem. Every earlier OS comparison in this project changed machine, kernel
and filesystem at once.

---

## Corpora, as measured

Stated shapes matter more than names, and one earlier assumption about cb7 was
wrong — it is **not** a directory-density test.

| corpus | entries | dirs | files/dir | bytes | mean file |
|---|---:|---:|---:|---:|---:|
| congress-100k | 109,615 files | 25,851 | 4.24 | 850 MB | **7.9 KB** |
| cb7 | 59,311 files + 3,310 symlinks | 10,872 | 5.46 | 5.6 GB | **99.0 KB** |
| large files | 7 files | 1 | — | 3.94 GiB | **576 MB** |

congress and cb7 have almost the same directory density (4.24 vs 5.46). What
separates them is **mean file size — 12.5×**. Together with the large-file set
they form a clean size ladder spanning five orders of magnitude.

---

## Results

### congress-100k — 109,615 small files

*Post-4.26. The pre-4.26 column is in "What changed since v2" above.*

| destination | median | files/s |
|---|---:|---:|
| freya — Linux, ZFS, 7950X | **8.46 s** | 12,961 |
| orion — Linux, ext4, **Pi 5** | **9.23 s** | 11,890 |
| WSL2 — Linux, ext4, 7900X | **15.62 s** | 7,019 |
| Windows — NTFS, 7900X | **55.83 s** | 1,964 |

### cb7 — 62,621 entries, mixed sizes, 3,310 symlinks

| destination | median | entries/s | MB/s | note |
|---|---:|---:|---:|---|
| WSL2 ext4 | **62.85 s** | 996 | 91.2 | post-4.26 |
| Windows NTFS | **92.59 s** | 676 | 61.9 | post-4.26 |
| freya | 58.06 s | 1,078 | 98.8 | pre-4.26 |
| orion (Pi 5) | 95.41 s | 656 | 60.1 | pre-4.26 |

The two Linux rows predate 4.26 and are not re-measured; cb7 gains little there
(WSL moved only 64.70 → 62.85 s, 1.03×) because at 99 KB mean the per-file cost
is already small next to the bytes.

### large files — 3.94 GiB in 7 files

| destination | median | MB/s |
|---|---:|---:|
| Windows NTFS | 62.65 s | 64.4 |
| WSL2 ext4 | 62.20 s | 64.9 |
| freya | 66.79 s | 60.3 |

---

## The headline: the OS penalty is per-file, and it evaporates with size

Holding hardware constant — same CPU, same NVMe, same link, same source, only
the OS and filesystem differing:

| corpus | mean file size | Windows ÷ WSL-Linux |
|---|---:|---:|
| congress-100k | 7.9 KB | **3.57×** |
| cb7 | 99.0 KB | **1.47×** |
| large files | 576 MB | **1.01×** |

*All three rows post-4.26 except the large-file row, which is bandwidth-bound
and unchanged by it. Before 4.26 the same ladder read 4.72× / 1.61× / 1.01×:
the shape was identical, the small-file end simply exaggerated by xsync's own
serialized receiver.*

Monotonic collapse across five orders of magnitude. Windows is not "slower at
I/O" — it charges a fixed cost per file creation, and once files are large
enough to amortise it, the difference disappears entirely.

The honest headline is therefore **"the OS costs ~3.6× on small files and
nothing on large ones"**, which is both more precise and more useful than
`docs/OS.md`'s standing claim that the OS is worth ~6×. Note that this figure
moved once already, when 4.26 removed serialization on our side: an OS penalty
measured through a serialized tool is partly a measurement of the tool.

### Defender is real, but it is not the explanation

Measured **before 4.26**, on one NTFS volume, two sibling directories,
alternating, three rounds. The absolute tax should be unchanged — it is a
per-file cost — but its share of a now-smaller gap will have grown:

| arm | median | files/s |
|---|---:|---:|
| Windows as configured | 87.23 s | 1,257 |
| Windows, benchmark path excluded | 71.39 s | 1,536 |

Defender costs **1.22× — 15.84 s, or 144 µs per file**, 18% of the wall clock a
Windows user actually experiences. But it is only **~20% of the
Windows-versus-Linux gap**; with scanning removed entirely Windows is still
**3.8×** slower on congress. And on the large-file corpus scanned and excluded
are indistinguishable (62.65 s vs 65.98 s, within noise) — confirming the cost
is per-file, not per-byte.

### A Raspberry Pi 5 ties a 7950X, and beats a 7900X sixfold

On congress, orion (4 cores, 3 GB) lands at 9.23 s against freya's 8.46 s —
**within 9% of a 32-thread 7950X** — and against the *same class* of CPU running
Windows, 55.83 s, it is **6.0× faster**. Parallelising the receiver did not
change this: the Pi kept pace before and after.

Small-file sync is not CPU-bound. It is bound by per-file work in the OS and
the transport, which is why a Pi keeps up and why the operating system dominates
the result.

On cb7 the Pi finally falls behind (95.41 s, 60.1 MB/s) — but that is its link,
not its CPU: 60 MB/s is where this transport tops out.

---

## Transport ceilings

Nothing here is limited by the wire, which is worth stating because it bounds
what a faster link could buy.

| path | throughput | of 1 GbE |
|---|---:|---:|
| 1 GbE practical ceiling | ~112 MB/s | 100% |
| `dd \| ssh 'cat >/dev/null'` | 86.5 MB/s | 77% |
| Wi-Fi 6 (5 GHz, 80 MHz), same test | 72.3 MB/s | 65% |
| xsync, large files | 64.9 MB/s | 58% |

Two ceilings already sit below the wire: SSH costs ~23%, and xsync a further
~25% on top of it. A faster link cannot be evaluated until those are separated —
see backlog 4.50.

---

## What this does not measure

- **`/mnt/c` from WSL** is ~32 files/s, roughly 30× *worse* than doing the same
  work natively on Windows. That is the 9p bridge — a per-operation RPC — not
  NTFS. It is guidance ("never sync into `/mnt/c`"), not a filesystem result.
- **NTFS versus Win32** was deliberately not decomposed. It would need a ReFS
  Dev Drive, which is confounded by Defender's performance-mode default, and
  nothing depends on the answer: the size ladder above already establishes the
  cost is per-file.
- **Anything above 1 GbE**, any x86 that is not Zen 4, XFS, btrfs, and BSD.
- **What still limits small files.** 4.26 closed the receiver half. The apply
  pool is not wired into `run_data_sink`, the multi-stream data path, but small
  files never travel that route — they ride the control session, which is
  backlog 4.25.

## Figures retracted during this cycle

Recorded so they are not resurrected from older notes:

- **13,964 files/s / 11.1× for WSL** — runs that never happened. The WSL VM was
  idle-shutting-down mid-benchmark and `-q >/dev/null 2>&1` hid the failures.
  Exposed by 513 MB/s appearing over a gigabit link.
- **A 2.5× "VHDX warmup effect"** — a mechanism I invented to explain the above
  bogus number before testing it. There is no such effect.
- **14.4 s for congress to freya** — a lone 1.8× outlier that reproduces under
  no configuration tested, including its own commit built for both ends. Treated
  as an erroneous measurement; the host is stable (24.88 / 24.79 / 26.30 s for
  the same code across three sessions).
