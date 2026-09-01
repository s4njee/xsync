# Nine days against thirty years

> This paragraph is the only human written content on this page. A few years ago, just having something this coherent following would have been amazing. Now the speech and the content itself is beyond what I think any reasonable person would have expected in such a short time. I've been spending the last few days trying to create an rsync clone, a file transfer tool, because it was taking a long time to transfer the Congress data files I used for my only solo project csearch.org. So I wanted to see if I could beat the tool I was using. Well I got close but no cigar; as of its current state, its within 10-20% of rsync speed if not closer most of the time, and in a few cases it does beat it, but not decisively. At this point I am losing interest, because in most cases, xsync _is_ fast enough and close to the speeds of the battle tested rsync. I believe in the next year or two, AI will be able to profile rsync better, and deliver better results, and likely beat rsync. But right now its not there. At least with Claude Opus 5. This is a fun space, and I am going to open it up for anyone else to explore. Here are the results, summarized by Claude below:

---

*Written 31 August 2026. Every figure below is drawn from measurements committed
to this repository.*

Agentic coding tools became broadly usable around March. Six months later, this
repository is nine days old, contains 36,732 lines of Rust across 131 commits,
and passes 360 tests. It is a file synchroniser. It is measurably as fast as
`rsync`, which was first released in 1996 and has had thirty years of
optimisation poured into it by people who understood the problem deeply.

That sentence is true and it is also the least interesting thing here, because
the second sentence is: **it is nowhere near feature parity, and the gap is not
closing at the same rate.**

## What was actually achieved

Performance parity, measured rather than claimed. Every figure below is a paired
ratio against `rsync -a` on the same hardware in the same session, arms
interleaved, three repetitions, exit status and landed file count verified per
run, and — where it matters — timed to durability rather than to process exit.

```chart
{"type":"hbar","title":"xsync throughput vs rsync -a (1.00 = parity, higher is xsync ahead)","unit":"x","height":330,"data":[{"name":"macOS local, 100k files","value":4.34},{"name":"macOS local, 10k files","value":2.47},{"name":"Linux local, cross-device","value":1.73},{"name":"Mac to mars, small files","value":1.02},{"name":"Mac to mars, large files","value":1.00},{"name":"mars to Mac, pull","value":1.02},{"name":"Mac to Pi 5, small files","value":1.06},{"name":"Mac to Pi 5, large files","value":0.81}]}
```

Local copies on APFS are 4.34x faster than rsync because the engine clones
subtrees instead of copying bytes. Over the network it is at parity on a
switched gigabit link in both directions. The one place it loses is a Raspberry
Pi receiving large files, and that deficit is now understood to the point of
being predictable: it equals *bytes ÷ the destination's write speed*, to within
2–11% across a fifteen-fold range of corpus size and two storage devices
differing 2.25x in speed.

An arbitrary stack, chosen up front with no benchmarking: Rust, BLAKE3 for
integrity, zstd with sample-and-skip compression, postcard framing over SSH.
None of it was selected because it was known to be fastest. It simply is.

## What was not achieved

```chart
{"type":"hbar","title":"rsync surface compared in the roadmap: 52 items","unit":"","height":200,"data":[{"name":"Missing","value":36},{"name":"Done","value":7},{"name":"Partial","value":6}]}
```

Sixty-nine percent of the compared rsync surface is missing. Among the missing
items is *delta transfer* — the rolling-checksum algorithm that is the reason
rsync exists and the thing its name refers to. Also missing: the daemon and its
whole authentication and module system, remote-to-remote transfers, and most of
the filter language.

So the honest summary is not "an agent rebuilt rsync in nine days". It is:

> An agent built a tool that **moves bytes as fast as rsync** in nine days, and
> would need considerably longer to accumulate the thirty years of edge cases,
> flags, and protocol surface that make rsync *rsync*.

Speed turns out to be the tractable part. Breadth is where the decades live.

## The part that was actually hard

Not the code. The measurement.

A single day of work on one code path — pulling files *from* a Raspberry Pi —
went like this:

```chart
{"type":"hbar","title":"One file-sync path, one day: pull throughput to a Pi 5 (MB/s, higher is better)","unit":" MB/s","height":230,"data":[{"name":"rsync (the target)","value":109.9},{"name":"after batching durability barriers","value":107.7},{"name":"after removing the lockstep","value":88.3},{"name":"after fixing a quadratic re-read","value":73.4},{"name":"morning baseline","value":29.7}]}
```

That is 3.6x in a day, and none of it was clever. The largest single win was
noticing that the server re-read and re-hashed an entire file for *every 8 MB
chunk it served* — 30 GB of reading to move 500 MB. The fix was to call a
function that already existed.

Over the same day, on the same code path, these published conclusions were found
to be wrong and retracted:

1. "2.5x faster than rsync" — a stale baseline; really **1.24x**.
2. "0.515x slower on local APFS" — measured before the clone path existed;
   really **2.47x faster**.
3. "a 1.31x gap on the Pi" — timed unfairly; really **1.20x**.
4. "local cost is flat in file count" — paired two different revisions.
5. "fsync frequency is the bottleneck" — measured: it is not.
6. "there is an ext4 gap" — there was not; rsync had left 2.4 GB unflushed.
7. "batching the flush will help" — measured zero, reverted.
8. "`sync_file_range` will help" — 0.7%, inside noise, reverted.
9. "a writer thread will help" — three designs, all null, all reverted.

Nine, in one day. Every one of them was a plausible, well-reasoned hypothesis
that measurement destroyed. Several had already been written into documentation
as fact before anyone checked.

The pattern is consistent and worth naming, because it is the actual lesson: **an
agent generates confident, coherent, wrong explanations at high speed.** It also
generates the experiments that kill them, if instructed to. The bottleneck on
quality is not the model's ability to write code or even to reason — it is
whether anything in the loop forces a claim to survive contact with a
measurement.

Two examples of how thin the margin is:

- A benchmark showed xsync losing to rsync on ext4. It was not losing. rsync
  returns with **2.4 GB still unwritten** in the receiver's page cache while
  xsync has already flushed. Timed to actual durability, xsync was ahead. Every
  wall-clock comparison in the project had been scoring the two tools on
  different work.
- A correction to a stale number introduced a *new* error of exactly the kind it
  was correcting: it paired a measurement from one revision with a measurement
  from another. The fix had to be fixed.

## Other things that fell out

**The tool is more careful than its author.** xsync flushes every chunk to disk
and journals it before acknowledging; rsync defers writeback and lets the kernel
catch up. That makes xsync look slower on a naive stopwatch and makes it
strictly safer on power loss. The default that reads as a performance bug is a
durability guarantee.

**Hardware config was worth more than code.** The Raspberry Pi's NVMe was
running at PCIe Gen2 because that is the default. One line in `/boot/config.txt`
took sequential writes from 414 to 765 MB/s and moved the ratio against rsync
from 0.806 to 0.861 — a larger single improvement than any code change attempted
that day.

**Some problems are not software problems.** After eliminating fsync cadence,
write pattern, receiver CPU (14% busy), receiver memory (xsync uses *less* than
rsync at 1.3 million files), sender platform, per-file cost, and three separate
writer-thread designs, the remaining explanation for the Pi's deficit is that its
NVMe and its ethernet share one PCIe fabric. An agent will happily write four
more thread pools before considering that.

**Negative results are the durable output.** The repository records what was
tried and failed — io_uring, multiplexed streams, hash parallelism, SSH cipher
selection, a C++ transport spike — with the measurements that closed each one.
That list is worth more than the code, because it is what stops the next
enthusiastic attempt.

## The claim, stated precisely

An AI agent, given six months of tooling maturity and nine days of wall time,
can produce a systems tool that matches a thirty-year-old C program on the
metric that program is famous for, on a stack nobody validated first.

It cannot yet produce the thirty years of surface area. And left unsupervised,
it will confidently document nine wrong conclusions a day while doing it.

Both halves of that are the story.
