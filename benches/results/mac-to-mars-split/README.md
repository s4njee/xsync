# split, Mac → mars: one rsync vs four, and what xsync already does

**One `xs --streams 4` beats four concurrent `rsync` processes.** The
four-instance workaround is unnecessary — the striping it simulates is already
in xsync, it is just not on by default.

- Source: Mac (M1 Max), `corpora/split` — 391,845 files, **2.07 GiB logical**
  across four top-level directories (congress sessions 115–118)
- Destination: mars, `~/split-dst-*` on the internal NVMe
- Link: Mac's only NIC is a USB gigabit adapter (`en8`, 1000baseT full duplex),
  practical ceiling ~118 MB/s
- No SSH multiplexing configured, so each rsync instance gets its own TCP
  connection — otherwise the parallel arm would have measured nothing
- 2 interleaved repetitions, fresh destination per run, teardown outside the
  timed window, remote `sync` **inside** it, all runs verified at 391,845 files
- Source page cache warmed before the first timed run

| arm | best | median | throughput (median) |
|---|---|---|---|
| rsync x1 | 57.31 | 59.37 s | 35.7 MiB/s |
| rsync x4 (one per directory) | 26.04 | 32.00 s | 66.2 MiB/s |
| xs `--streams 1` | 42.31 | 44.03 s | 48.1 MiB/s |
| **xs `--streams 4`** | **22.50** | **23.75 s** | **89.2 MiB/s** |
| xs `--streams 8` | 23.57 | 24.28 s | 87.3 MiB/s |

`rsync x4` was the noisiest arm (26.04 against 32.00, a 23% spread), so prefer
the best-vs-best comparison: **xs `--streams 4` 22.50 s against rsync x4
26.04 s, 1.16x ahead**. On medians it is 1.35x. Single-stream xsync is 1.35x
ahead of single rsync.

`--streams 8` is slightly *worse* than 4, so the remaining gap to the ~118 MB/s
link ceiling is not a stream-count problem.

Note the earlier figure of "3.1 GB" for this corpus came from `du`, which
reports allocated blocks; with 391,845 small files that overstates the logical
size by 50%. Throughputs above use the true 2.07 GiB.

## Why four rsyncs help so much

A single rsync moves 35.7 MiB/s on a link that can carry ~112. It is not
bandwidth-bound — it is bound by per-file serialization. Four independent
processes overlap each other's stalls, and reach 66.2.

The four instances finish within ~2.5 s of each other every run
(115:22.4 116:23.2 117:25.0 118:25.0), despite a 34% spread in file count
between the smallest and largest directory. So the split is not the interesting
variable; the concurrency is.

## What xsync already does

`sync_push_server_streams` stripes **whole small files** round-robin across the
data sessions, balanced by *count* rather than bytes, because per-file cost
dominates payload at this size:

```rust
let mut shares: Vec<Vec<FileEntry>> = (0..streams).map(|_| Vec::new()).collect();
for (index, file) in small_files.iter().enumerate() {
    shares[index % streams].push(file.clone());
}
```

Large files are *additionally* split by byte range across the same sessions. So
`--streams` does both things — whole-file distribution for small files, range
splitting for large ones. This corpus is 100% small files, so it exercises the
first path only.

## What is actually missing

The striping above runs over a **complete** `small_files` vector: the scan and
plan finish before any data moves. Four rsync instances instead overlap
traversal with transfer, each starting to send as soon as it has found
something.

That is the one structural advantage the four-process workaround still has, and
it is the same shape as O-3 in `OPTIMIZE.md` (the local path materialises every
task before starting). Making the scanner feed the stream queues incrementally
would close it.

## The open question: should the default change?

`DEFAULT_REMOTE_STREAMS = 1`. On this link that costs a default user 44.03 s
where 23.75 s was available — **1.85x left on the table**.

The conservative default is not obviously still justified. The code comment
warning that `--streams` was "30x slower than a single stream to a Raspberry
Pi" describes a bug that has since been fixed: one synchronous ack per
directory, which on congress is nearly one round trip per file. Directory
creation is now pipelined and that comment is the fix's own explanation.

**Not yet tested:** streams against orion, which is the host the pathology was
originally observed on, and against a high-latency link. Do that before
changing the default.
