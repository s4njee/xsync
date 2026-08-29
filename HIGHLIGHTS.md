# HIGHLIGHTS

A guided tour of what the benchmarks actually found.

[BENCHMARKv2.md](BENCHMARKv2.md) is the primary record — six studies across four
machines, each dated, self-contained, and written to be re-checkable.
[TUNING.md](TUNING.md) adds the local-copy baseline that motivated the work. Both
are dense on purpose. This page is not.

It is written to be read top to bottom. Each section builds on the one before it,
and the technical detail increases as you go — the first half needs no background
beyond "computers copy files."

**Contents**

1. [The one thing to understand first](#1-the-one-thing-to-understand-first)
2. [How fast is it? Two very different answers](#2-how-fast-is-it-two-very-different-answers)
3. [The `--streams` story: a bug found, diagnosed, and fixed](#3-the---streams-story-a-bug-found-diagnosed-and-fixed)
4. [How many workers? The rule everyone uses is wrong](#4-how-many-workers-the-rule-everyone-uses-is-wrong)
5. [The hardware had opinions too](#5-the-hardware-had-opinions-too)
6. [Four ways these benchmarks nearly lied](#6-four-ways-these-benchmarks-nearly-lied)
7. [What to actually do](#7-what-to-actually-do)
8. [Known bugs and open questions](#8-known-bugs-and-open-questions)
9. [Where this evidence stops](#9-where-this-evidence-stops)

---

## The 60-second version

- Copying **many small files is not a bandwidth problem.** It is a bookkeeping
  problem, and almost every result below follows from that one fact.
- **Over a network, `rsync -az` still wins** (11.0 s vs xsync's 17.3 s on 110k
  small files). Its edge is compression applied to the chatter, not just the data.
- **Copying between local drives, xsync wins**: 1.76x faster than rsync, 3.0x
  faster than `cp`.
- The `--streams` flag was a **12x slowdown**. The cause was found, fixed, and
  re-measured at **2.70x faster** than before. It is still only worth ~1.06x on
  mixed data, so the default of one stream stays.
- The industry-standard rule "**use one worker per CPU core**" is **wrong**. On a
  4-core Raspberry Pi, using 16 workers was 20% faster than using 4.
- **Warm caches invalidated two earlier conclusions.** Measuring cold changed the
  answers, not just the numbers.

---

## 1. The one thing to understand first

The main test corpus is 110,000 files from the US Congress — bills, amendments and
votes — spread across 26,000 folders. It totals 557 MB, and the **median file is
about 2 KB**. Over 92% of the files are smaller than 16 KB.

That shape matters more than the size:

> On a gigabit network, 557 MB of raw data should cross the wire in about
> **5 seconds**. The fastest tool measured took **11 seconds**. The slowest took
> **354 seconds** — nearly six minutes for the same 557 MB.

The network was never the constraint. It was idle most of the time. What costs
time is the *per-file* work: ask about a file, create it, write it, close it, set
its timestamp, confirm it, move to the next one. Do that 110,000 times and the
overhead is the entire job.

**This is the lens for everything below.** When a result looks strange — a faster
drive losing to a slower one, more parallelism making things worse — it is almost
always because the thing being measured is bookkeeping, not throughput.

The same lens explains why this project exists at all. An early local-copy test
([TUNING.md §3](TUNING.md)) found xsync copying **zero bytes** — the filesystem's
clone feature let it skip the data entirely — and *still* losing to rsync, because
it was spending 1.9 ms of kernel time per file against rsync's 0.41 ms. When you
are doing nothing per item, doing it ten thousand times is still the whole cost.

---

## 2. How fast is it? Two very different answers

### Over a network: rsync wins, and it is worth knowing why

Mac to a Linux box over gigabit ethernet, same 110k-file corpus, median of 3 runs:

| Tool | Time | vs. best |
|---|---:|---:|
| `rsync -az` | **11.0 s** | 1.00x |
| `rsync -a` | 14.6 s | 1.33x |
| **`xs` (default)** | **17.3 s** | 1.57x |
| `tar` + zstd over ssh | 26.4 s | 2.40x |
| `tar` plain over ssh | 30.2 s | 2.76x |
| `scp -r` | 353.5 s | 32.2x |

Three things worth pulling out:

**rsync's advantage is compression in an unexpected place.** The `-z` flag is
worth a consistent 25%. These are JSON and XML files, so they compress well — but
the real win is that rsync compresses the *conversation* between the two machines,
not just the file contents. On a workload this chatty, the conversation is a large
share of the bytes.

**`tar` has no protocol overhead at all and still loses.** It is one continuous
stream with zero per-file negotiation, and it is 2.4x slower than rsync. The
bottleneck is the receiving end unpacking 110,000 files one at a time. rsync does
the same work but overlaps it with the transfer; the `tar` pipeline waits for
whichever end is slower. Compressor choice barely matters — and `zstd -19` is
actively harmful (43.8 s), because it makes the compressor the bottleneck to save
bandwidth that was never scarce.

**`scp` is not in the race.** Modern `scp` asks a question and waits for an answer
for every single file. At 110,000 files that is most of the six minutes.

**Where xsync lands:** third, ahead of every `tar` variant, and with the *steadiest*
timings of anything measured — 0.9 s of spread across three runs, versus 3.2 s for
`rsync -az`. It is 1.57x off the leader, and the most plausible route to closing
that gap is compressing protocol chatter the way rsync's `-z` does.

<details>
<summary><b>Aside: why <code>rsync -axhP</code> is slower than <code>rsync -az</code></b> (+5.2 s) — click to expand</summary>

A commonly-used flag combination, decomposed against the same corpus:

| Change | Cost |
|---|---:|
| Adding `-P` (progress) | **+3.03 s** |
| Dropping `-z` (compression) | **+1.96 s** |
| `-x` and `-h` together | +0.22 s (noise) |

`--progress` emits about 144 bytes per file — roughly **15.8 MB of progress text**
across 110,000 files, versus literally zero for `-a`. Notably, writing to a real
terminal was not measurably slower than writing to a pipe, so the 3 s is rsync
*generating* the text, not the terminal drawing it. In a real terminal emulator,
which must parse and redraw all 15.8 MB, the true cost is very likely higher.

If you want progress without paying for it, `--info=progress2` prints one
self-updating line and costs **+0.95 s** instead of +3.03 s.
</details>

### Between local drives: xsync wins clearly

Same class of corpus, copied NVMe-to-NVMe on a 32-core Linux workstation. The
timing includes a `sync()` so the data is genuinely on the drive, not sitting in
memory pretending to be:

| Tool | Time | vs. xsync |
|---|---:|---:|
| **`xs --local-workers 16`** | **1.95 s** | 1.00x |
| `xs` (default) | 2.01 s | 1.03x |
| `tar c \| tar x` | 2.01 s | 1.03x |
| `rsync -a` | 3.43 s | 1.76x |
| `cp -a` | 5.89 s | 3.02x |

**1.76x faster than rsync, 3.0x faster than `cp`.** This is xsync's design point,
and it is the headline local result.

The result worth staring at, though, is `tar`: a plain single-threaded pipe ties
xsync's 32-worker default. Whatever serial bottleneck remains inside xsync, a
simple sequential stream reaches the same place with none of the machinery. That
is a standing item on the list, and it is worth more than further worker tuning.

---

## 3. The `--streams` story: a bug found, diagnosed, and fixed

This is the most instructive thread in the whole document, because it runs the
full arc: a shocking number, a plausible explanation that turned out to be wrong,
a real diagnosis, a fix, and an honest re-measurement showing the fix mattered
less than hoped.

### Act 1 — a flag that made things 12x slower

`--streams 8` opens eight parallel network connections instead of one. It should
help. On the congress corpus it was catastrophic:

| | Time | Files/sec |
|---|---:|---:|
| `xs` (default, 1 stream) | 17.3 s | 6,351 |
| `xs --streams 8` | 211.5 s | 518 |

Reproduced cleanly across three runs. The first written explanation was that the
parallel streams were fighting each other for a shared resource.

### Act 2 — that explanation was wrong, and one measurement settled it

If streams were contending, the cost should grow as you add more of them. It does
not:

| Mode | Time | Files/sec |
|---|---:|---:|
| 1 stream | 0.35 s | 3,074 |
| `--streams 2` | 4.19 s | 257 |
| `--streams 4` | 4.42 s | 243 |
| `--streams 8` | 4.42 s | 243 |

**The entire 12x penalty is paid going from one stream to two.** Quadrupling from
2 to 8 costs another 5%. That is not contention; that is a different code path.

And it was. Turning on multiple streams silently switched to a **separate,
older implementation** that had never received the batching and pipelining work
the main path had. It sent each small file individually and waited for an
acknowledgement before sending the next — about three network round trips per
2 KB file. The extra connections did nothing at all for small files except spawn
more `ssh` processes.

This also explained a result that had looked inexplicable: turning compression
off made no difference in multi-stream mode. It turned out compression was never
on in that mode to begin with — the connection carrying the small files
negotiated no compression support. There was nothing for `--no-compress` to
switch off.

*The general lesson: a hypothesis that predicts "cost scales with N" was refuted
by varying N. Profiling was never needed.*

### Act 3 — where the flag genuinely helps

Before fixing anything, it was worth checking the opposite case. On **large**
files, `--streams` does exactly what it was designed to do:

| 1.4 GB across 7 large files | Time | MB/s |
|---|---:|---:|
| 1 stream | 16.8 s | 82 |
| `--streams 2` | 14.9 s | 93 |
| **`--streams 4`** | **14.4 s** | **96** |
| `--streams 8` | 16.8 s | 82 |

A single `ssh` connection on this link tops out at 106 MB/s. Single-stream xsync
moved data at 83 MB/s — **78% of what the wire allows** — because it was limited by
its own per-stream work (reading, hashing, framing, writing). Four streams hit
106 MB/s exactly: the flag parallelized xsync's own CPU work until the network
became the limit, then stopped helping because there was nothing left to win.
Eight streams is *worse* than four — nine connections competing for a saturated
link.

So the honest headline is **~1.2x, not more**. Going from 78% to 100% of the link
is the entire prize on gigabit. On a faster network, where a single stream would
leave far more of the wire idle, the flag would matter considerably more.

### Act 4 — the fix, measured on a mixed corpus

The batched, pipelined sender was extracted into one function used by *both* code
paths — the duplication was exactly how they had diverged in the first place.

Tested on a real software build tree (59,311 files, 5.5 GB), which is the most
honest test available because it stresses both paths at once: 83% of the files are
under 8 KB, yet 68% of the bytes live in just 78 large files.

| | Time | MB/s |
|---|---:|---:|
| before the fix | 149.3 s | 38 |
| **after the fix** | **55.3 s** | **102** |

**2.70x faster.** Worth noting: a projection from the earlier per-file rate
predicted the "before" number would be around 300 s. It was 149 s. The projection
was wrong, which is precisely why it was measured rather than estimated.

### The verdict

With the flag fixed, here is what it is actually worth:

| Corpus shape | Best mode | Gain over 1 stream |
|---|---|---:|
| All small files | 1 stream | — |
| Mixed (83% small files, 68% large bytes) | `--streams 8` | 1.06x |
| All large files | `--streams 4` | 1.17–1.23x |

The gain tracks the share of bytes flowing through the large-file machinery,
which is exactly what the design predicts. **The default of one stream remains
correct.** On the mixed corpus, two streams is still a 6% *regression*.

One honest caveat that applies to all of the above: one stream and many streams
are different code paths, not the same path at a different width. These tables
compare two implementations, not the isolated effect of parallelism.

---

## 4. How many workers? The rule everyone uses is wrong

"Workers" are the parallel threads doing local file work. The conventional default
— used here too — is one worker per CPU core. Three studies took that apart.

### First answer (wrong): scaling stops at 12

On the 32-core workstation, adding workers stopped helping at 12. Everything from
12 to 32 was the same number. This looked like a serial bottleneck somewhere, and
a task was filed to hunt for it.

### The correction: warm caches were hiding the work

That test read from a 557 MB corpus on a machine with 61 GB of RAM. The data was
entirely in memory. Re-run with caches deliberately dropped, on the full
**1.3 million file** corpus:

| Same config, only cache state differs | Time | Files/sec |
|---|---:|---:|
| cold (honest) | 101.6 s | 12,980 |
| warm (measuring RAM) | 44.6 s | 29,570 |

**Warm caches were hiding more than half the work** — and they had inverted the
conclusion. Cold, throughput improves monotonically all the way to 32 workers.
There was no plateau at 12.

The reason is clean: with a warm cache there is no waiting on the drive, so the
run is limited by CPU and locking, and extra threads have nothing useful to do.
Cold, every file carries real device latency, and more workers hide more of it.

*The workers are not there to compute faster. They are there to have more requests
in flight so the drive is never idle.* That distinction is the key to the next
result.

### The real test: an 8x smaller machine

The 32-core box could not settle whether the ideal worker count *tracks the core
count* or just happens to be ~32 on that storage — both numbers were 32. So the
USB drive holding the corpus was physically carried over to a **Raspberry Pi 5**:
4 cores, 3 GB of RAM. With 16 GB of data against 3 GB of RAM, caching is
impossible by construction — the measurements are honest whether you want them to
be or not.

Copying 1.3 million files on the Pi:

| Workers | Time | vs. 4 workers (= core count) |
|---:|---:|---:|
| 1 | 458.3 s | 0.57x |
| 2 | 336.4 s | 0.77x |
| **4** (what the core-count rule picks) | 260.0 s | 1.00x |
| 8 | 230.2 s | 1.13x |
| 16 | 218.3 s | **1.19x** |
| 32 | 216.0 s | **1.20x** |

**Going to 4x the core count is worth 20%.**

Now put the two machines side by side:

| Host | CPU cores | Best worker count |
|---|---:|---:|
| Workstation | 32 | 32 |
| Raspberry Pi 5 | 4 | 16–32 |

**An 8x difference in core count produced no meaningful difference in the
optimum.** Both machines wanted 16–32 workers. The 32-core machine matched the
core-count rule by pure coincidence.

Two details from the Pi confirm the mechanism. Scaling is already sub-linear
*below* the core count — 4 workers on 4 cores buys 1.76x, not 4x. And at the best
setting, the Pi uses only **21% of the drive's write capacity** with a load average
near 2 on a 4-core box. It is waiting, not computing, at every single point on
that curve.

**Conclusion: the right worker count is set by how many requests the storage will
service at once, not by how many cores the host has.** Core count is at best a
weak proxy.

**What this does *not* license:** raising the floor everywhere. macOS caps workers
at 4 because additional workers measurably contend there, so the same change that
gains 20% on the Pi could lose elsewhere. And 16 concurrent writers suits an NVMe
drive; it would likely thrash a spinning disk.

---

## 5. The hardware had opinions too

Three findings that are really about storage, not about xsync — but that anyone
benchmarking file copies will run into.

**Some SSDs are unreliable to benchmark on.** One destination drive produced times
swinging between 61 s and 94 s for an identical configuration. Another produced
164.61 / 164.81 s — a spread of 0.1%. The difference is the NAND type: the erratic
drive is QLC, which keeps a fast write buffer whose size depends on how much
you have written recently. Its speed is a function of its own history. (An earlier
note in the benchmark file blamed the filesystem for this; that attribution was
corrected.)

**ZFS costs 45% to read a million small files.** Same destination, same corpus,
same worker count — only the source filesystem differs:

| Source filesystem | Time | Files/sec |
|---|---:|---:|
| ext4 | 53.6 s | 24,609 |
| ZFS (with lz4 compression) | 77.9 s | 16,931 |

And ZFS was reading *fewer physical bytes* — 9.3 GB compressed against 16 GB on
ext4. The cost is per-file overhead: checksums, cache bookkeeping,
decompression. At 2 KB per file, that overhead is the whole story. Neither drive
was anywhere near its bandwidth limit.

**The slower direction was the one *reading* from the external drive.** External →
internal took 70.1 s; internal → external took 53.6 s — even though the internal
drive is far faster on paper. Cold reads of a million small files are
latency-bound, and USB adds per-request latency that parallelism only partly
hides. Both directions ran at roughly 160–210 MB/s against a 979 MB/s link,
confirming that even here the job stayed metadata-bound.

---

## 6. Four ways these benchmarks nearly lied

This is the part that generalizes beyond this project. Every one of these was
caught and fixed *before* the numbers were published, and each would have produced
a confidently wrong result.

**1. The benchmark almost measured rsync twice under two names.** The remote
machine had no `xs` installed, and xsync's transport setting silently falls back to
rsync when the remote binary is missing. Two rows of the table would have been the
same program. Caught by requiring the xsync handshake to appear in the session log
before any timing was recorded.

**2. macOS `tar` inflated its own results by 2.2x.** Apple's `tar` emits hidden
sidecar files for extended attributes. Linux `tar` unpacks them as *real files* —
so archives of 109,615 files arrived as 245,081, inflating both the payload and
the time (83.6 s instead of 27.6 s). It is invisible if you inspect the archive on
macOS, which quietly re-merges the sidecars. Every run is now verified by counting
files and directories at the destination.

**3. Warm caches inverted a conclusion.** See §4 — this did not just shift numbers,
it produced a plateau that does not exist and sent work down a wrong path.

**4. A busy machine produced phantom results.** One test host runs Kubernetes and
monitoring; its load average wandered between 3 and 11 mid-session. An early sweep
showed alarming outliers at specific worker counts that simply vanished on
re-measurement. The fix was **interleaved A/B pairs** — run the two arms
alternately rather than one after the other — which is the only design that
survives a machine drifting underneath you.

The general practice these add up to: **verify the output, not just the timer.**
Count the files that arrived. Confirm the program you think you ran is the one that
ran. Drop the caches. Interleave the arms. Report the spread, not just the median.

---

## 7. What to actually do

**Copying between drives on one machine** — this is where xsync is strongest.
Defaults are fine; it will beat `rsync -a` by ~1.75x and `cp -a` by ~3x.

**Copying over a network** — xsync works and is competitive, but `rsync -az` is
still faster on many small files. Use `-z`; it is worth 25% on compressible data
and costs nothing you will notice.

**Leave `--streams` alone** unless you are moving large files over a fast link.
The default of 1 is correct for small and mixed workloads. On a corpus of genuinely
large files, `--streams 4` is worth about 1.2x on gigabit — and would be worth
more on a faster network.

**Do not use `--streams 16`.** It fails outright (see below).

**If you are tuning worker counts,** tune against your *storage*, not your CPU
count — and measure with caches dropped, or you will measure your RAM.

**If you want rsync progress output,** use `--info=progress2` rather than `-P`. It
costs about a third as much.

---

## 8. Known bugs and open questions

**Open bug: `--streams 16` fails with `Broken pipe`.** xsync opens one SSH
connection per stream plus one for control, so 17 concurrent connections — above
OpenSSH's default early-drop threshold of 10. This is the likely cause but is
unconfirmed. Either way the failure is opaque and arrives *after* several
connections have already reported success.

**Fixed: small-file streams.** The unpipelined multi-stream path is gone; one
shared implementation now serves both. 2.70x on the mixed corpus.

**Fixed: single-file sources with `--streams`.** Any multi-stream transfer of a
single file previously failed with a "Not a directory" error, after the transfer
had already reported success. A path was being appended to itself.

**Fixed: a 16% tax on every local transfer.** A pre-transfer safety check was
performing two system calls per file — 219,230 of them on one run — serially, on a
single thread, while 16 workers sat idle. Parallelizing it took the cost from
16.4% to about 1%. It still does every one of those calls; it just stopped doing
them one at a time. (Correctness was the hard part: the check's results depend on
processing order in one respect, so the parallel version chunks the work and merges
in order, with a test asserting it matches the serial result exactly at 2, 3, 8 and
16 workers.)

**Open question: what is the remaining serial bottleneck?** A single-threaded `tar`
pipe still ties xsync's 32-worker default on local copies. Finding that is worth
more than any further worker tuning.

**Open question: compressing protocol chatter.** rsync's 1.57x network lead comes
substantially from compressing the conversation, not just the payload. That is the
clearest available route to closing the gap.

---

## 9. Where this evidence stops

Stated plainly, because it bounds every number above:

- **All destinations were empty.** Every measurement is a cold, complete, first-time
  copy. Nothing here says anything about *re-syncing* a directory that has only
  partly changed — which is rsync's and xsync's actual design point, and where the
  interesting comparison probably lives.
- **The network tests are one corpus, one link, one direction.** The
  many-small-files shape is close to the worst case for per-file protocols. A
  corpus of large files would reorder that table substantially, and the `tar`
  pipelines in particular should do far better.
- **A few rows are single runs**, not medians — noted in the source file. Their
  ordering is not in question, but their exact values are softer.
- **`--streams` comparisons are across two code paths**, not one path at two
  widths.
- **The worker-count conclusion rests on two machines.** They differ by 8x in core
  count, which is what makes the refutation of the core-count rule solid. What
  should replace that rule — and how it interacts with spinning disks, with macOS,
  and with network transfers — is not yet settled by measurement.

For the full data, the exact commands, the machine specifications, and the
reasoning behind each conclusion, read [BENCHMARKv2.md](BENCHMARKv2.md).
