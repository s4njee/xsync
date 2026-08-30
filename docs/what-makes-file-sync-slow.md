# What actually makes file sync slow

*Notes from a week of measuring xsync — a file-sync tool — across four machines,
three operating systems and three very different piles of files.*

We set out to answer what sounds like a simple question: **when copying a lot of
files over a network, what is the slow part?** The CPU? The disk? The network?
The operating system?

The answer turned out to be "it depends, and the thing it depends on is the
average file size" — but getting there meant discarding a lot of measurements,
including several we had already written down as facts.

---

## The combinatorial problem

Here is the trouble with benchmarking a tool that copies files from one computer
to another. Every run has, at minimum:

- a **sending** machine (CPU, disk, network card)
- a **receiving** machine (same again)
- an **operating system** at each end
- a **filesystem** at each end
- a **network** between them
- a **corpus** — the actual files, which vary in count, size and shape
- and the **tool's own settings** — parallelism, compression, buffer sizes

Four machines, three operating systems, three corpora and a handful of tuning
knobs is already thousands of combinations. You cannot run them all, and if you
run a scattered handful you will get a scattered handful of numbers that
disagree with each other for reasons you cannot name.

Worse, the confounds are invisible. During this work we recorded, and later had
to throw away:

- a receiver that was "2.5× faster" — the runs had **silently failed**, and the
  tool printed a plausible time anyway
- a whole benchmark matrix taken while the **house network had degraded** to 2%
  of its normal speed
- a baseline that was 1.8× faster than anything we could reproduce afterwards,
  on the same machine with the same code

Every one of those looked like a real result at the time. Two of them looked
specifically like *our code had gotten slower*, which is the most expensive kind
of false alarm because it sends you hunting through a diff.

What eventually worked was boring discipline:

1. **Verify every run.** Check the exit status *and* count the files that
   actually arrived. If the count is wrong, it is not a slow run, it is a failed
   run, and it must never be reported as a time.
2. **Throw away the first run.** The first copy into a fresh destination is
   systematically slower — we measured 23 s cold against 12 s settled for the
   same work.
3. **Alternate arms in one sitting.** Never compare today's number against one
   from yesterday. Run A, B, A, B in the same session.
4. **When something looks like a regression, build both versions and race
   them.** Every single time we did this, the code was innocent and the
   environment had changed.

---

## Controlling for the hardware

The first real question was whether Windows is genuinely slower than Linux at
this, or whether the Windows machine simply had different hardware.

Comparing a Windows desktop against a Linux server tells you nothing — different
CPU, different disk, different network card, different everything. The trick is
**WSL2**: Windows Subsystem for Linux runs a real Linux kernel *on the same
machine*. So you can copy the same files, from the same sender, over the same
cable, to the same NVMe drive, and change **only the operating system and
filesystem**.

That is as close to a controlled experiment as this gets. And the answer is
unambiguous:

```chart
{
  "type": "hbar",
  "title": "Same machine, same disk, same cable — 109,615 small files",
  "unit": "s",
  "height": 210,
  "data": [
    { "name": "WSL2 (Linux, ext4)", "value": 15.6, "color": "success" },
    { "name": "Windows (NTFS)", "value": 51.9, "color": "danger" }
  ]
}
```

Same hardware. **3.3× slower on Windows.** Nothing about the CPU, the disk or
the network can explain it, because none of them changed.

---

## The hypothesis we had to reject: antivirus

The obvious explanation is Windows Defender. It inspects every file as it is
written, and we are writing a hundred thousand of them. Surely that is the
whole story.

It is not. We measured it directly — same disk, two folders, one of them added to
Defender's exclusion list, runs alternated:

```chart
{
  "type": "hbar",
  "title": "Windows Defender: real, but not the explanation",
  "unit": "s",
  "height": 240,
  "data": [
    { "name": "Windows, as configured", "value": 87.2, "color": "danger" },
    { "name": "Windows, Defender excluded", "value": 71.4, "color": "amber" },
    { "name": "Linux, same machine", "value": 18.7, "color": "success" }
  ]
}
```

Defender costs **1.22×** — about 144 microseconds per file, or 18% of the time a
Windows user actually experiences. That is a real cost and worth knowing.

But look at the gap it needs to explain. Turning Defender off entirely closes
roughly **20% of the distance** to Linux. The remaining **3.8×** is still there,
on the same machine, with scanning disabled.

There is a second, cleaner proof. Defender's cost is *per file*, not per byte.
When we ran the same test with a handful of very large files instead of many
small ones, the scanned and excluded runs became indistinguishable — 62.7 s
against 66.0 s, within noise, and in the wrong direction. Scanning seven files
costs a millisecond.

So: antivirus is a genuine tax, and it is not why Windows is slow.

---

## The result that surprised us most: a Raspberry Pi keeps up

This one we re-ran several times because we did not believe it.

Sending 109,615 small files to four different receivers:

```chart
{
  "type": "hbar",
  "title": "Files received per second — the hardware barely matters",
  "unit": "/s",
  "height": 260,
  "data": [
    { "name": "Linux desktop (7950X, 32 threads)", "value": 12961, "color": "success" },
    { "name": "Raspberry Pi 5 (4 cores, 3 GB)", "value": 11890, "color": "accent" },
    { "name": "WSL2 on Windows desktop (7900X)", "value": 7019, "color": "amber" },
    { "name": "Windows on that same desktop", "value": 1964, "color": "danger" }
  ]
}
```

A **Raspberry Pi 5** lands within 9% of a 32-thread Ryzen 9 7950X. It beats the
same-class desktop CPU running Windows by **six times**.

A £60 computer with four small cores and 3 GB of memory keeps pace with a
desktop that costs twenty times as much — and the *only* thing that separates
the two bottom rows is which operating system booted.

If small-file sync were limited by processing power, this chart would be
impossible. It is telling you, loudly, that the work is somewhere else.

We later varied the **sending** machine too, and got the same message from the
other direction. Sending the same files from a Pi 5, an M1 Max laptop and a
32-thread desktop:

```chart
{
  "type": "hbar",
  "title": "Sender barely matters for small files (109,615 files, to WSL2)",
  "unit": "s",
  "height": 210,
  "data": [
    { "name": "From 7950X desktop", "value": 14.2, "color": "success" },
    { "name": "From M1 Max laptop", "value": 15.6, "color": "accent" },
    { "name": "From Raspberry Pi 5", "value": 17.0, "color": "amber" }
  ]
}
```

Three machines spanning an enormous range of capability, spread across **20%**.
Meanwhile changing the *receiver* on one machine cost 3.3×.

---

## How fast can a gigabit cable actually go?

Before blaming anything, it is worth knowing the ceiling. "Gigabit Ethernet"
sounds like 125 MB/s. In practice you never see that, and it matters where the
losses go:

```chart
{
  "type": "hbar",
  "title": "Where the bandwidth goes on a 1 GbE link",
  "unit": " MB/s",
  "height": 250,
  "data": [
    { "name": "Theoretical 1 GbE", "value": 125, "color": "muted" },
    { "name": "Realistic ceiling", "value": 112, "color": "muted" },
    { "name": "Through SSH (encrypted)", "value": 86.5, "color": "accent" },
    { "name": "Wi-Fi 6, same test", "value": 72.3, "color": "amber" },
    { "name": "xsync, large files", "value": 64.9, "color": "success" }
  ]
}
```

Two things stand out.

**SSH costs about 23%.** Encryption is not free, and a single SSH stream is a
single-threaded encryption job.

**Wi-Fi 6 came within 16% of a wired connection.** The wired link here runs
through a USB Ethernet adapter, and that adapter — not the cable — appears to be
the weaker component. Worth remembering before assuming wired is always better.

The practical conclusion: at **64.9 MB/s** the tool is running at 58% of the
wire. There is no point buying a faster network until the two ceilings below it
are addressed — a 10× faster cable would mostly measure OpenSSH.

---

## Small files and large files are different problems

This is the part that reframed everything. Here is the Windows-versus-Linux
penalty plotted against the average size of the files being copied:

```chart
{
  "type": "line",
  "title": "The OS penalty collapses as files get bigger",
  "height": 300,
  "x": { "log": true, "domain": [5, 1000000], "label": "mean file size (KB, log scale)" },
  "y": { "min": 0, "max": 4, "label": "Windows ÷ Linux, same machine" },
  "series": [
    { "name": "Windows penalty", "color": "danger",
      "points": [[7.9, 3.32], [99, 1.47], [589824, 1.01]] }
  ]
}
```

At 8 KB average, Windows takes **3.3× longer**. At 99 KB, **1.5×**. At 576 MB
per file, **1.01×** — the difference vanishes entirely.

Windows is not "slower at input/output". It charges a **fixed cost every time a
file is created**, and once files are big enough that moving their contents
dominates, that fixed cost disappears into the noise.

This also explains why the two workloads need opposite tuning:

- **Many small files:** the bottleneck is per-file bookkeeping — creating,
  naming, permissioning and renaming each one. Parallelism helps. Compression
  helps. Bandwidth is irrelevant; a small-file transfer runs at a fraction of
  the wire speed and does not care.
- **A few large files:** the bottleneck is bytes on the wire. Parallelism buys
  nothing because one stream already saturates the link. The only things that
  matter are the encryption cost and the cable.

And the two halves of the tool are limited by different machines entirely.
Varying sender and receiver independently across both corpora:

```chart
{
  "type": "hbar",
  "title": "Who is the bottleneck? It depends on the corpus",
  "unit": "×",
  "height": 260,
  "data": [
    { "name": "Small files — swap receiver", "value": 3.33, "color": "danger" },
    { "name": "Small files — swap sender", "value": 1.20, "color": "muted" },
    { "name": "Large files — swap sender", "value": 1.90, "color": "accent" },
    { "name": "Large files — swap receiver", "value": 1.47, "color": "muted" }
  ]
}
```

**Per-file work is paid by the receiver. Per-byte work is paid by the sender.**

For small files, swapping the receiver costs 3.3× while swapping the sender
costs 1.2×. For the byte-heavy corpus it inverts: the sender spread widens to
1.9× — because now every sender is limited by its own encryption throughput,
and the Pi's weaker cryptography finally shows.

---

## So is it CPU-bound, link-bound, or OS-bound?

All three, depending on the corpus — and for the interesting case, none of the
usual suspects.

**Large files: link-bound**, but not by the cable. Every platform converges
around 62–66 s for the same 3.9 GB, within 7% of each other. That is not the
network's 112 MB/s ceiling; it is SSH's 86.5 MB/s, and then the tool's own
overhead on top. Windows, Linux and macOS are indistinguishable here because
none of them is the limit.

**Small files: not CPU-bound.** The Pi 5 result settles it. Four small cores
keep pace with thirty-two large ones.

**Small files: not link-bound either.** A small-file transfer runs at roughly
half the wire speed and is unaffected by having more of it.

**Small files: OS-bound, and tool-bound.** This is the honest answer, and the
second half of it is uncomfortable. Some of what we first measured as "the
operating system's fault" turned out to be *our own tool serialising work*.

Two changes fixed that:

```chart
{
  "type": "hbar",
  "title": "Removing serialisation, one end at a time",
  "unit": "×",
  "height": 235,
  "data": [
    { "name": "Sender: read ahead while sending", "value": 2.05, "color": "success" },
    { "name": "Receiver: publish files in parallel", "value": 1.62, "color": "success" },
    { "name": "Spread small files across streams", "value": 1.12, "color": "accent" }
  ]
}
```

The sender used to read up to 8,192 files with the network sitting idle, then
compress and send them with the disk sitting idle — one thread alternating
between two resources, using neither well. The receiver, meanwhile, decoded
*and* wrote every file on a single thread.

Fixing both took the same transfer from 26.3 s to 8.5 s — **3.1× faster** — with
no change to the hardware, the network or the operating system.

And here is the uncomfortable part: **the measured OS penalty shrank when we
fixed our own code.** It read 4.7× before the receiver fix and 3.3× after. An
operating-system comparison run through a serialised tool is partly a
measurement of the tool. We had published the larger number first.

---

## Why is Windows slower? An honest guess

we want to be clear that this section is speculation. We measured *that* Windows
charges a large fixed cost per file creation; we did not prove *why*.

What the evidence constrains:

- It is **per file, not per byte** — it vanishes on large files.
- It is **not antivirus** — excluding Defender closes only a fifth of the gap.
- It is **not the hardware** — same machine, same disk, same cable.
- It is **not relieved by more parallelism**. This is the strongest clue.
  Adding concurrent writers on Linux helps; on Windows it actively *hurts*:

```chart
{
  "type": "line",
  "title": "More parallel streams: helps on Linux, hurts on Windows",
  "height": 300,
  "x": { "domain": [1, 8], "ticks": 4, "label": "parallel connections" },
  "y": { "min": 0, "label": "time relative to a single stream" },
  "series": [
    { "name": "Linux", "color": "success", "points": [[1, 1.0], [2, 0.84], [4, 0.76], [8, 0.78]] },
    { "name": "Windows", "color": "danger", "points": [[1, 1.0], [2, 1.12], [4, 1.17]] }
  ]
}
```

On Linux, four connections cut the time by a quarter. On Windows, two
connections make it 12% *worse* and four make it 17% worse.

That pattern — a fixed per-creation cost that gets worse rather than better
under concurrency — is what you would expect from **serialisation inside the
filesystem layer itself**. Some resource is being taken exclusively per file
creation, so adding writers adds contention instead of throughput. Plausible
candidates include NTFS's master file table, the directory entry structures, or
the stack of filter drivers every file operation traverses on Windows (antivirus
is only one such filter; there are usually several).

A reasonable next experiment would compare NTFS against ReFS on the same Windows
machine, which would separate "the filesystem" from "the Windows I/O stack". We
have deliberately not done it: ReFS volumes get different antivirus treatment by
default, which would reintroduce the confound we just spent effort eliminating,
and no decision currently depends on the answer.

---

## What we would do next

**Investigate a real bug first.** The Raspberry Pi copies files *out* perfectly
but fails partway through receiving 109,615 of them, with the connection timing
out. It is the only machine here with 3 GB of memory, and the receiver now
starts a pool of worker threads. That smells like a genuine defect rather than a
flaky machine, and it is the only outright failure in the whole matrix.

**Get a faster link — but only afterwards.** The tool runs at 58% of a gigabit
cable, so a faster network would mostly measure SSH. The interesting version of
this experiment is not "is it faster" but "which ceilings move" — a fixed
per-operation cost will not improve with bandwidth, which is exactly how you
tell the two apart. The desktop already has a 2.5 Gb port, so this costs about
£25 and a cable rather than a new network.

**Look at the encryption cost.** SSH takes 23% off the top, and a single
connection is a single-threaded cipher. That is the largest identified,
unaddressed loss in the whole stack.

**Widen the corpus.** Everything here rests on three collections of files. The
size ladder is a strong result precisely because it spans five orders of
magnitude, but we would like to know what happens with incompressible data, or
with the deep, wide directory trees that a `node_modules` folder produces.

**Test on a slow CPU, and on Wi-Fi.** Every x86 machine measured is a recent
high-end Ryzen. "Processing power does not matter" is a claim we can only
defend on fast processors, which is a suspicious place to be making it.

---

## The three lessons that generalise

**Measure the thing you are actually changing.** Most of our early numbers varied
several things at once. WSL was valuable precisely because it holds everything
constant except the one variable in question.

**A plausible number is not a result.** The single most useful change we made was
refusing to record any run that did not verify its own output. Failures print
plausible times. Degraded networks print plausible times. Only the file count
tells the truth.

**Your own tool is part of the measurement.** The OS penalty we confidently
reported dropped by a third once we stopped serialising work inside the program.
Before attributing a slow result to somebody else's software, it is worth
checking how much of it belongs to yours.
