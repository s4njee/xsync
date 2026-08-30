# Anatomy of a slow file sync

*Profiling xsync — a Rust rsync replacement — across four hosts, three
operating systems and five orders of magnitude of file size. What follows is
the measurement log, the three false conclusions I published before catching
them, and the two serialisation bugs that were hiding inside what I had
labelled "the OS".*

---

## The system under test

xsync pushes a directory tree to a remote host over SSH. The client scans and
plans locally; the remote runs `xs --server` on the other end of stdio. Files
below `MAX_DATA_SEGMENT` (8 MB) are coalesced into batches — a `FileBatch`
metadata frame followed by one `FileSegment` per file — and larger files are
split into 8 MB chunks that can be striped across parallel SSH connections.
Every frame carries a BLAKE3 digest verified before commit, and the receiver
acknowledges a file only after it is durably renamed into place.

Three corpora, chosen to span the interesting axis:

| corpus | entries | dirs | files/dir | bytes | mean file |
|---|---:|---:|---:|---:|---:|
| congress | 109,615 files | 25,851 | 4.24 | 850 MB | **7.9 KB** |
| cb7 | 59,311 files + 3,310 symlinks | 10,872 | 5.46 | 5.6 GB | **99.0 KB** |
| large | 7 files | 1 | — | 3.94 GiB | **576 MB** |

Note the shapes. congress and cb7 have nearly identical directory density — I
had assumed cb7 was a directory-metadata stress test and it is not. What
separates them is mean file size, 12.5×. That accident turned out to be the
most useful thing about the corpus set.

---

## Why this matrix is a trap

The parameter space is sender OS × receiver OS × filesystem × corpus × stream
count × worker count × link. You cannot sample it uniformly, and an unprincipled
sample produces numbers that disagree for reasons you cannot attribute.

That is the boring problem. The interesting problem is that **the confounds are
silent**. Three results I recorded as fact and later had to retract:

**A 2.5× "warmup effect" that did not exist.** WSL runs were coming in at a
suspiciously consistent 7.85 s. I had run them as `xs … -q >/dev/null 2>&1`
without checking `$?`. The WSL VM was idle-shutting-down mid-transfer, every run
was failing, and the failure path took a stable ~7.85 s. Worse, I then *explained*
the bogus number — "the first large write expands the dynamically-allocated ext4
VHDX" — a mechanism that sounds right and is entirely fictional. What exposed it
was arithmetic: the same runs implied 513 MB/s over a gigabit link.

**A whole benchmark matrix taken on a degraded LAN.** Mid-session, throughput to
one host fell to 1.7 MB/s against an 86.5 MB/s baseline. Every cell collected in
that window looked like a plausible slowdown. A router reboot restored it.

**A baseline that never reproduced.** One host measured 14.4 s for work that
consistently took 24–26 s afterwards. I first wrote this up as *host drift* —
which would taint every number ever taken there. Building the exact commit for
both client and server and re-running gave 24.98 / 24.46 s: the code was
identical, the host was stable, and the original figure was simply wrong.

Two of those three initially looked like **my code had regressed**, which is the
most expensive false positive available — it sends you bisecting a diff that
contains nothing.

The discipline that survived contact:

```
1. A run is invalid unless exit == 0 AND landed entry count == corpus count.
   Report INVALID; never report a time.
2. Discard the first run of every batch.  Cold: 23.2/21.8/21.0 s.
   Settled: 11.6/11.9/11.4 s.  Same work, 1.9x.
3. Alternate arms within one session.  Never compare across sessions.
4. On any suspected regression, build both commits for BOTH ends and race them.
   Every single time I did this, the code was innocent.
```

Rule 4 has now fired three times with a 100% false-positive rate on the code.

---

## Isolating the OS with WSL2

`docs/OS.md` carried a standing claim that "the OS is worth ~6×", derived from a
7900X under Windows doing 1,099 files/s against 6,046 on Linux. That comparison
crosses machine, kernel, filesystem and network simultaneously — it is not an OS
measurement, it is a *fleet* measurement.

WSL2 fixes this. It runs a real Linux kernel on the same silicon, so you can hold
CPU, NVMe, link and source tree constant and vary only the OS/filesystem pair.

```chart
{
  "type": "hbar",
  "title": "congress-100k to one machine, varying only OS + filesystem",
  "unit": "s",
  "height": 230,
  "data": [
    { "name": "WSL2 — Linux, ext4 (VHDX)", "value": 15.6, "color": "success" },
    { "name": "Windows — Win32, NTFS", "value": 51.9, "color": "danger" }
  ]
}
```

**3.32×**, same box. There is a third path worth mentioning and then dismissing:
`/mnt/c` from inside WSL, which reaches NTFS through a 9p transport. It runs at
~32 files/s — 30× *worse* than doing the same work natively on Windows. That row
measures the 9p RPC bridge, not NTFS, and it is guidance ("never sync into
`/mnt/c`") rather than a filesystem result.

---

## Rejecting the antivirus hypothesis

Real-time scanning is the obvious suspect: a filter driver in the write path,
invoked per file, on a workload that is nothing but per-file work.

Test: one NTFS volume, two sibling directories, one added via
`Add-MpPreference -ExclusionPath`, arms alternated over three rounds. Scoping to
a sibling directory rather than toggling Defender globally keeps the setting
change out of the measurement.

```chart
{
  "type": "hbar",
  "title": "Defender: 1.22x, and ~20% of the Windows-vs-Linux gap",
  "unit": "s",
  "height": 240,
  "data": [
    { "name": "Windows, as configured", "value": 87.2, "color": "danger" },
    { "name": "Windows, path excluded", "value": 71.4, "color": "amber" },
    { "name": "Linux, same NVMe", "value": 18.7, "color": "success" }
  ]
}
```

15.84 s across 109,615 files is **144 µs/file** — 18% of the wall clock a Windows
user actually sees, and worth documenting. But the gap it needs to explain is
68.53 s. Scanning is **23% of it**. With Defender fully excluded Windows still
runs **3.82×** slower on identical hardware.

The confirming test is the large-file corpus: 3.94 GiB in 7 files gives 62.65 s
scanned against 65.98 s excluded — indistinguishable, and in the wrong
direction. 144 µs × 7 is a millisecond. **The cost is per-creation, not
per-byte**, which also means it scales with exactly the thing that makes Windows
slow, and is not itself the cause.

---

## The Pi anomaly

```chart
{
  "type": "hbar",
  "title": "congress-100k receive rate, post-optimisation",
  "unit": "/s",
  "height": 265,
  "data": [
    { "name": "Ryzen 9 7950X, 32 threads, ZFS", "value": 12961, "color": "success" },
    { "name": "Raspberry Pi 5, 4 cores, 3 GB, ext4", "value": 11890, "color": "accent" },
    { "name": "WSL2 on 7900X, ext4", "value": 7019, "color": "amber" },
    { "name": "Windows on that 7900X, NTFS", "value": 1964, "color": "danger" }
  ]
}
```

A Pi 5 receives within **9%** of a 32-thread 7950X, and beats the same-class
desktop CPU running Windows by **6.0×**. This is the single most informative
data point in the set, because it falsifies the CPU hypothesis outright: if
per-file work were compute-bound, an 8× core-count difference and a large IPC
difference could not vanish into 9%.

I then varied the *sender* deliberately — same corpora, same receivers, three
very different machines pushing:

| sender | congress → WSL2 | congress → Windows | cb7 → WSL2 | cb7 → Windows |
|---|---:|---:|---:|---:|
| 7950X | **14.21 s** | **50.45 s** | **61.76 s** | **95.84 s** |
| M1 Max | **15.62 s** | **51.94 s** | **66.00 s** | **97.10 s** |
| Pi 5 | **17.03 s** | **54.00 s** | **117.16 s** | **142.06 s** |
| *spread* | *1.20×* | *1.07×* | *1.90×* | *1.48×* |

```chart
{
  "type": "hbar",
  "title": "Sender vs receiver influence, by corpus",
  "unit": "×",
  "height": 265,
  "data": [
    { "name": "small files — swap receiver", "value": 3.33, "color": "danger" },
    { "name": "small files — swap sender", "value": 1.20, "color": "muted" },
    { "name": "large-byte — swap sender", "value": 1.90, "color": "accent" },
    { "name": "large-byte — swap receiver", "value": 1.47, "color": "muted" }
  ]
}
```

**Per-file work is charged to the receiver; per-byte work to the sender.** On
congress the sender spread is 1.07–1.20× across wildly different machines while
swapping the receiver costs 3.33×. On cb7 it inverts, and the rates say exactly
why: freya sustains 92.8 MB/s, macOS 86.9 MB/s (at its measured 86.5 MB/s `ssh`
ceiling), the Pi 48.9 MB/s against a 53–62 MB/s link. On the byte-heavy corpus
every sender is bandwidth-bound, so the ordering is just each host's SSH
throughput and the Pi's weaker AES shows.

---

## What a gigabit link actually yields

```chart
{
  "type": "hbar",
  "title": "Transport ceilings, measured on the same host pair",
  "unit": " MB/s",
  "height": 255,
  "data": [
    { "name": "1 GbE theoretical", "value": 125, "color": "muted" },
    { "name": "1 GbE practical", "value": 112, "color": "muted" },
    { "name": "dd | ssh 'cat >/dev/null'", "value": 86.5, "color": "accent" },
    { "name": "Wi-Fi 6 (5 GHz, 80 MHz)", "value": 72.3, "color": "amber" },
    { "name": "xsync, 7 x 576 MB files", "value": 64.9, "color": "success" }
  ]
}
```

Two ceilings sit below the wire before xsync gets a turn. **SSH costs ~23%** — a
single connection is a single-threaded cipher — and xsync a further ~25% on top.
At 64.9 MB/s we are at **58% of the link**.

Wi-Fi 6 landing within 16% of "wired" is a hint that the wired path here runs
through a **USB gigabit adapter**, and that adapter is the weak component. It is
also my leading suspect for a 5.3 ms in-session SSH round trip — absurd for a
LAN, and load-bearing for the tuning below.

The practical conclusion is that faster cabling is currently pointless. A 10 GbE
link would mostly benchmark OpenSSH. The useful version of that experiment is
not "is it faster" but "which ceilings move" — a fixed per-operation cost does
not scale with bandwidth, and that is precisely how you separate it from a
bandwidth cost.

---

## Small files and large files are different programs

```chart
{
  "type": "line",
  "title": "Windows ÷ Linux (same box) against mean file size",
  "height": 300,
  "x": { "log": true, "domain": [5, 1000000], "label": "mean file size, KB (log)" },
  "y": { "min": 0, "max": 4, "label": "OS penalty" },
  "series": [
    { "name": "penalty", "color": "danger",
      "points": [[7.9, 3.32], [99, 1.47], [589824, 1.01]] }
  ]
}
```

Monotonic collapse over five orders of magnitude. Windows is not slower at I/O;
it charges a fixed cost per file creation which amortises to nothing once files
are large. Everything about tuning follows from which regime you are in.

### Regime 1: per-file. Four bugs, all serialisation.

**The sender was phase-separated.** `send_small_files_batched` looked like this:

```rust
while cursor < small_files.len() {
    let mut loaded = Vec::new();
    while /* batch not full */ {
        loaded.push(source_reader.read(&file)?);   // PHASE A: up to 8192 blocking reads
    }
    for (index, (_, data)) in loaded.into_iter().enumerate() {
        let digest = blake3::hash(&data);          // PHASE B: hash, compress, frame
        write_data_frame_buffered(writer, id, &seg, compress, level)?;
    }
}
```

Phase A blocks on the disk with the socket idle; phase B saturates a core with
the disk idle. One thread, two resources, neither used well — and it shows up as
**~50% CPU on both endpoints simultaneously**, which is the signature of a
lockstep pipeline rather than a busy one. A loader thread now runs one batch
ahead over a bounded channel. **2.05×**, verified across three sessions.

**The receiver applied inline.** The decode loop must stay serial — it is an
ordered frame stream — but `write temp → verify → set metadata → rename` is
independent per file. An `ApplyPool` of `min(cores, 8)` threads now does that
off the decode thread.

The ack contract is what makes it safe: a file is acknowledged only after the
rename, so crash semantics are unchanged. Acks now complete out of order, which
was *already* legal because the sender's `drain_acks` counts acknowledgements
and never matches them to ids.

The trap was a deadlock, found by a hanging test. The sender drains to **zero**
at every batch boundary. If the receiver blocks on its next read while files are
still in flight, it is holding acks the sender is waiting for and both sides
stop. The receiver has the signal it needs without a protocol change:

```rust
// active_files empties exactly when a batch completes
let limit = if active_files.is_empty() { 0 } else { apply.capacity() };
apply.collect(limit, |id| { acks_unflushed = true; self.ack_buffered(writer, id, 4) })?;
```

Full drain at the boundary, free overlap within the batch. **Windows 1.62×,
freya 1.46×, Pi 1.32×, WSL 1.21×** — the gain tracks how receiver-bound the
platform is.

**Streams bought nothing.** `--streams N` opens N extra SSH connections, but the
partition sent everything ≤ 8 MB down the *control* session. For congress that
is 100% of the corpus: N connections, zero parallelism on the workload that is
slowest. Striping small-file batches across the data sessions fixed it — worth
only **1.12×** now, because the receiver pool had already removed the starvation
that would have made it look impressive.

**Syscalls the planner did not need.** The dropped-metadata preflight re-`stat`ed
every planned file purely because `uid`/`gid`/`nlink` did not survive the
planning record encoding. Carrying them in the record removed it:

```
                statx    newfstatat    total
  before       516,060      161,339   677,399
  after        406,445      161,339   567,784
```

Exactly **109,615** fewer — the file count, to the digit — and 16.2% of all
stat-family syscalls. Two independent methods (in-process counters and
`strace -f -c`) agreed exactly.

### Regime 2: per-byte. Almost nothing to tune.

Large files converge within 7% across Windows, WSL, macOS and Linux, because
none of them is the limit — SSH and the tool's chunk path are. Parallel streams
buy nothing; one connection already saturates what the cipher will give you.
Compression is the only lever, and only when the data is compressible.

### The pipeline window, and why RTT set it

One tuning result worth isolating. The sender pipelines up to
`MAX_PIPELINED_FRAMES` before draining acks, and it used to drain to *half* the
window:

```rust
if outstanding >= MAX_PIPELINED_FRAMES {          // was 256
    writer.flush()?;
    drain_acks(decoder, reader, &mut outstanding, MAX_PIPELINED_FRAMES / 2)?;  // was /2
}
```

At 256, congress-100k takes ~856 of those stalls, each paying the measured
5.3 ms in-session round trip. Raising the window to 2048 and draining to ¾ gave
**1.26×**; it is flat at 8192 and 16384, so 2048 is the knee.

Two things about that. First, an 8 KB `BufWriter` default against a **5,327-byte
mean frame** meant the buffer held barely one frame, so removing a per-frame
`flush()` in isolation did nothing measurable — the two changes only work
together. Second, **the knee is a function of the link's RTT**, and that 5.3 ms
is suspect. If the USB NIC is responsible, 2048 is tuned to an artifact.

```chart
{
  "type": "hbar",
  "title": "congress-100k to a Linux receiver, cumulative",
  "unit": "s",
  "height": 230,
  "data": [
    { "name": "before sender overlap", "value": 26.3, "color": "danger" },
    { "name": "+ sender read-ahead", "value": 12.8, "color": "amber" },
    { "name": "+ receiver apply pool", "value": 8.5, "color": "success" }
  ]
}
```

**3.11× end to end**, no hardware changed.

---

## So what is it bound by?

**Large files: transport-bound, not link-bound.** Every platform lands at
62–66 s for the same 3.94 GiB. That is not the 112 MB/s wire; it is 86.5 MB/s of
SSH and then ~25% of tool overhead.

**Small files: not CPU-bound.** The Pi settles it — four Cortex cores keep pace
with 32 Zen 4 threads.

**Small files: not link-bound.** congress runs at ~54 MB/s against an 86.5 MB/s
ceiling and does not care about the difference.

**Small files: OS-bound *and* tool-bound**, and the split moved under
measurement. The OS penalty read **4.72×** before the receiver pool and
**3.32×** after. A third of what I had attributed to Windows was xsync
serialising its own receiver and amplifying Windows' per-creation cost.

That is the uncomfortable general lesson: **an OS comparison conducted through a
serialised tool is partly a measurement of the tool**, and I published the larger
number first.

---

## Why Windows is slow: bounded speculation

I measured *that* Windows charges a large fixed cost per file creation. I did
not establish *why*. What the evidence constrains:

- **Per-creation, not per-byte** — it vanishes at 576 MB mean.
- **Not antivirus** — excluding Defender closes 23% of the gap.
- **Not hardware** — same CPU, NVMe, link, source.
- **Not relieved by concurrency.** This is the strongest constraint:

```chart
{
  "type": "line",
  "title": "Parallel connections: Linux scales, Windows anti-scales",
  "height": 300,
  "x": { "domain": [1, 8], "ticks": 4, "label": "parallel SSH connections" },
  "y": { "min": 0, "label": "time ÷ single-stream time" },
  "series": [
    { "name": "Linux", "color": "success", "points": [[1, 1.0], [2, 0.84], [4, 0.76], [8, 0.78]] },
    { "name": "Windows", "color": "danger", "points": [[1, 1.0], [2, 1.12], [4, 1.17]] }
  ]
}
```

Linux: 4 connections → 0.76×. Windows: 2 → 1.12×, 4 → 1.17×. Adding writers
makes it *worse*, and this is with the receiver already applying across 8 threads
per connection.

A fixed per-creation cost that anti-scales under concurrency is what you expect
from **serialisation inside the file-creation path**, not from a slow device.
Candidates, in rough order of my confidence: MFT record allocation and the
`$Bitmap`/`$MFT` metadata locks; directory index (B-tree) updates taking an
exclusive lock per insert; and the filter-driver stack — every `NtCreateFile`
traverses a chain of minifilters, of which Defender is only one, and each adds
fixed overhead that no amount of parallelism removes.

The experiment that would separate "NTFS" from "the Windows I/O stack" is
NTFS vs ReFS on the same box, via a Dev Drive. **I deliberately did not run
it**: Dev Drives get different antivirus treatment by default, which
reintroduces exactly the confound I spent a day eliminating, and no engineering
decision currently depends on the answer — the size ladder already tells us the
cost is per-creation, which is the actionable part.

---

## Roadmap

**A real bug first.** The Pi sends fine but fails partway through *receiving*
109,615 files, with the SSH connection timing out — before and after a reboot,
while healthy on every other metric. It is the only 3 GB host, and the receiver
now spawns a worker pool. That correlation is strong enough that I would treat
it as a memory-pressure defect in the apply path rather than a flaky machine.

**Attack the SSH ceiling.** 23% off the top, single-threaded cipher, and it is
the largest identified unaddressed loss. Options: `ControlMaster` multiplexing,
cipher selection per host (the Pi has no AES acceleration), or moving the data
plane off SSH entirely for the v2 daemon.

**2.5 GbE point-to-point, then re-derive the knee.** The desktop already has a
2.5 GbE NIC negotiated down to 1 Gb. A USB-C adapter and a direct cable is ~£25
and no switch. The goal is not throughput — it is to re-measure the 5.3 ms RTT
without the USB NIC, because `MAX_PIPELINED_FRAMES = 2048` was tuned to that
number and may be fitted to an artifact.

**Corpus coverage.** Three corpora, and congress compresses 4.2×, which flatters
every wire-bytes figure. Incompressible small files and `node_modules`-shaped
trees are the obvious gaps.

**Slow CPUs and lossy links.** Every x86 host here is Zen 4 and every link is
sub-millisecond wired. "Not CPU-bound" is a claim I can currently only defend on
fast CPUs.

---

## Three things worth stealing

**Verify the invariant, not the exit code.** Checking `$?` alone would not have
caught the degraded-LAN matrix. Checking landed entry count against corpus count
catches failures, partial transfers, and misconfigured destinations in one
assertion — and it is what caught the 513 MB/s impossibility.

**A mechanism proposed to explain a surprising number must be tested before it is
written down.** My fictional VHDX-expansion story was plausible, internally
consistent, and load-bearing for a published headline. The number it explained
was from runs that never completed.

**Instrument the thing you are changing, not the thing that is easy to read.**
`wire_bytes` — the counter I reached for to measure a metadata-compression
change — turned out to exclude metadata frames entirely, so an A/B of
compression-on and compression-off returned byte-identical totals. The metric
was blind to precisely the feature it was meant to measure. Measuring it
properly required instrumenting the encoder: 19 frames, 9,957,243 → 1,255,326
bytes, **7.93×** — against a commit message that had predicted 15.5×.
