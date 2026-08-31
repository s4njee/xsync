# Benchmarks v3 — the picture after 4.15 and 4.26

Measured 2026-08-30, in one session, on one client, with every run verified.
Supersedes nothing in `BENCHMARKv2.md`; that file keeps the tuning history and
the long tail of experiments. This one answers a narrower question: **now that both
halves of the small-file path have been unserialized — the sender (4.15) and
the receiver (4.26) — what is actually slow, and why?**

---

> **rsync comparisons are not like-for-like unless they measure time-to-durable.**
> rsync leaves ~2.4 GB of a 4.32 GiB transfer unwritten in the receiver's page
> cache at exit; xsync leaves ~2.6 MB, because it `sync_data`s each chunk.
> Transfer time therefore flatters rsync. Measured to durability, xsync is at
> 113.5 MB/s against rsync's 109.9 (4.64).

> **Read `docs/network-topology.md` before comparing hosts.** Mac and mars
> share an ethernet switch; freya and orion sit behind a ~800 Mbit/s mesh
> backhaul. Measured 2026-08-31: switched 904 Mbit/s at 1.0 ms RTT, mesh
> 792 Mbit/s at 3.9 ms. Throughput ceilings taken against freya or orion are
> mesh ceilings, not gigabit ones.

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

---

## Against rsync

`BENCHMARKv2.md` recorded "rsync wins, and compression is why" — `rsync -az` at
11.0 s against an xsync that was 1.57× behind it. That was before the sender
overlap (4.15), the receiver pool (4.26) and small-file striping (4.25). The
picture has changed, but not uniformly, and the exceptions are the interesting
part.

All figures congress-100k unless noted, two reps, warmup discarded, verified.

| sender → receiver | rsync -a | rsync -az | xsync | best |
|---|---:|---:|---:|---|
| macOS → freya | 20.24 s | 24.76 s | **8.00 s** | xsync **2.5×** |
| macOS → freya, *no-op re-sync* | 6.52 s | — | **2.07 s** | xsync **3.2×** |
| macOS → freya, *cb7* | 80.2 s | 195.3 s | **59.0 s** | xsync **1.36×** |
| freya → orion | 9.86 s | 8.08 s | **7.81 s** | xsync **1.03×** |
| orion → freya | **9.41 s** | 16.60 s | 10.67 s | **rsync -a 1.13×** |

### Three things this says

**The sender platform decides the margin.** xsync is 2.5× ahead from macOS, at
parity from a fast Linux box, and slightly *behind* from a Pi. rsync's cost
profile is far more sender-sensitive than ours: macOS rsync is remarkably slow
here, and that — not xsync being fast — is most of the 2.5×.

**`-z` is a liability, not a feature, outside compressible data.** rsync
compresses unconditionally when asked. On cb7 — largely `node_modules` and
already-compressed assets — `-az` costs **2.4×** over `-a` (195 s against 80 s).
On the Pi it costs **1.8×** (16.6 s against 9.4 s) because zlib outruns the CPU.
xsync's sample-and-skip heuristic avoids both traps automatically, which is
where a good part of the cb7 and Pi-receiver margins come from.

**The one loss is not compression.** rsync -a beating xsync from the Pi looked
like our default compression taxing a weak CPU. It is not:
`xsync --no-compress` measured **10.68 s** against the compressed **10.70 s** —
indistinguishable. The remaining ~13% is elsewhere, and the honest note is that
xsync computes and verifies a BLAKE3 digest for every file, which `rsync -a`
does not do at all. We are doing strictly more work and landing within 13%.


---

## Does the sender matter? A 3 × 3 matrix

Every result above used the same macOS client, so "the OS is worth 3.6×" was
really a statement about *receivers*. Varying the sender deliberately — an
M1 Max, a 32-thread 7950X and a 4-core Pi 5, each pushing the same two corpora
to three receivers — separates the two roles. Two of the receivers are the same
physical machine, differing only in OS.

Two reps each, warmup discarded, every run verified.

**congress-100k** — 109,615 files, 850 MB, mean 7.9 KB

| sender | → orion (Pi 5) | → WSL2 ext4 | → Windows NTFS |
|---|---:|---:|---:|
| freya (7950X) | 7.45, 7.57 → **7.51 s** | 14.24, 14.19 → **14.21 s** | 50.12, 50.77 → **50.45 s** |
| macOS (M1 Max) | 9.34, 9.39 → **9.37 s** | 15.70, 15.54 → **15.62 s** | 52.00, 51.88 → **51.94 s** |
| orion (Pi 5) | — | 17.03, 17.03 → **17.03 s** | 51.60, 56.40 → **54.00 s** |
| *sender spread* | *1.25×* | *1.20×* | *1.07×* |

**cb7** — 62,621 entries, 5.6 GB, mean 99 KB

| sender | → orion (Pi 5) | → WSL2 ext4 | → Windows NTFS |
|---|---:|---:|---:|
| freya | 75.81, 75.40 → **75.61 s** | 62.21, 61.31 → **61.76 s** | 96.58, 95.11 → **95.84 s** |
| macOS | 103.24, 98.59 → **100.92 s** | 65.92, 66.07 → **66.00 s** | 98.10, 96.11 → **97.10 s** |
| orion | — | 117.98, 116.35 → **117.16 s** | 142.45, 141.67 → **142.06 s** |
| *sender spread* | *1.33×* | *1.90×* | *1.48×* |

**The Pi is the fastest receiver in the fleet on small files** — 7.51 s from
freya, against 8.46 s for freya receiving from the same sender. On cb7 it drops
behind, but that is its 62 MB/s link rather than its CPU.

### The sender scales with bytes; the receiver scales with file count

On congress the sender is nearly irrelevant — **1.07× to 1.20×** across three
very different machines — while swapping the *receiver* from WSL to Windows on
one box costs **3.33×**. A Pi 5 sends a hundred thousand small files about as
fast as a 7950X does.

On cb7 that inverts. The sender spread widens to **1.90×** and the receiver
effect narrows to **1.47×**, because cb7 moves 6.7× the bytes. The rates say
why: to WSL, freya sustains 93 MB/s, macOS 87 MB/s — at its measured
`ssh` ceiling of ~86.5 MB/s — and orion only 49 MB/s, consistent with the
53–62 MB/s SSH throughput measured on that link. **Every sender is
bandwidth-bound on cb7**, so the ranking is simply each machine's SSH
throughput, and the Pi's weaker crypto shows.

The rule that falls out: **per-file work is paid by the receiver, per-byte work
by the sender.** Small-file corpora are a receiver benchmark; large-byte corpora
are a sender-and-link benchmark. It is why 4.26 (the receiver) beat 4.15 (the
sender) on congress, and why the OS ratio collapses as files get bigger.

### orion as a receiver: the failure was our harness

It looked like the Pi could not *receive* a large transfer — mid-transfer
`ssh: … Operation timed out`, before and after a restart, while sending fine.
Filed as a probable memory-pressure defect in the apply pool, since it is the
only 3 GB host.

**It was a stale IP.** The router reboot moved orion from `.218` to `.217` by
DHCP and the benchmark script had the old address hardcoded. Resolving by name
instead, orion receives congress-100k in **8.9 s** across four consecutive runs,
and a killed-client test confirmed the server exits cleanly rather than
orphaning processes. It is the **fastest receiver in the fleet**:

| sender → orion | congress | cb7 |
|---|---:|---:|
| freya (7950X) | **7.45, 7.57 s** | 75.81, 75.40 s |
| macOS (M1 Max) | 9.34, 9.39 s | 103.24, 98.59 s |

The harness now resolves every host by name.

### A caution this matrix earned

Its first run was collected while the LAN was degraded — 1.7 MB/s to Windows,
9.4 to freya, 28 to orion, against an 86.5 MB/s baseline. Every one of those
numbers was discarded after a router reboot restored freya to 89.6 MB/s. In
between, a receiver measuring 17 s against a remembered 9 s looked exactly like
a regression in 4.25, and an A/B of the 4.25 and 4.26 server builds on that host
(15.5 s vs 17–18 s) is what proved the code innocent. **Third time this cycle
that an environment change impersonated a code change.**
