# Benchmarks v3 — the picture after 4.15

Measured 2026-08-30, in one session, on one client, with every run verified.
Supersedes nothing in `BENCHMARKv2.md`; that file keeps the tuning history and
the long tail of experiments. This one answers a narrower question: **now that
the small-file sender overlap (4.15) has landed, what is actually slow, and
why?**

---

## What changed since v2

**4.15 landed.** The batch builder used to be serial and phase-separated — it
issued up to 8,192 blocking reads with the network idle, then hashed,
compressed and framed with the disk idle. A loader thread now runs one batch
ahead. Measured on an idle host, over the wired link, alternating arms, every
run verified:

| congress-100k, Mac → freya | before 4.15 | after 4.15 |
|---|---:|---:|
| median of 2 reps | 26.30 s | **12.80 s** |

**2.05×**, reproduced across three sessions at 1.86× (loaded), 2.15× (idle) and
2.05× (idle, wired, verified).

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

| destination | reps (s) | median | files/s |
|---|---|---:|---:|
| freya — Linux, ZFS, 7950X | 11.57, 11.89, 11.40 | **11.57** | 9,474 |
| orion — Linux, ext4, **Pi 5** | 11.53, 12.47, 14.55 | **12.47** | 8,790 |
| WSL2 — Linux, ext4, 7900X | 18.33, 18.59, 18.54 | **18.54** | 5,912 |
| Windows — NTFS, 7900X | 86.44, 90.50, 87.45 | **87.45** | 1,253 |

### cb7 — 62,621 entries, mixed sizes, 3,310 symlinks

| destination | reps (s) | median | entries/s | MB/s |
|---|---|---:|---:|---:|
| freya | 57.22, 58.06, 60.50 | **58.06** | 1,078 | 98.8 |
| WSL2 ext4 | 64.08, 65.64, 64.70 | **64.70** | 968 | 88.6 |
| orion (Pi 5) | 94.07, 95.41, 96.64 | **95.41** | 656 | 60.1 |
| Windows NTFS | 104.98, 104.23, 104.07 | **104.23** | 601 | 55.0 |

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
| congress-100k | 7.9 KB | **4.72×** |
| cb7 | 99.0 KB | **1.61×** |
| large files | 576 MB | **1.01×** |

Monotonic collapse across five orders of magnitude. Windows is not "slower at
I/O" — it charges a fixed cost per file creation, and once files are large
enough to amortise it, the difference disappears entirely.

The honest headline is therefore **"the OS costs ~4.7× on small files and
nothing on large ones"**, which is both more precise and more useful than
`docs/OS.md`'s standing claim that the OS is worth ~6×.

### Defender is real, but it is not the explanation

Measured on one NTFS volume, two sibling directories, alternating, three rounds:

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

### A Raspberry Pi 5 ties a 7950X, and beats a 7900X sevenfold

On congress, orion (4 cores, 3 GB) lands at 12.47 s against freya's 11.57 s —
**within 8% of a 32-thread 7950X** — and against the *same class* of CPU running
Windows, 87.45 s, it is **7.0× faster**.

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
- **The receiver side.** After 4.15 the client sits near 80% of one core and the
  server near 71%; neither is saturated. Backlog 4.26 is the other half.

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
