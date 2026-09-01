# The orion deficit is a receiver property, and it is not the flush cadence

`large1gb` (847 MB, 4 files), three interleaved repetitions, measured to
durability, content verified. Binary `2823c2c246cc-dirty` on every endpoint.

## Sender platform does not matter

| route | sender | ext4 | tmpfs |
|---|---|---:|---:|
| Mac → orion | macOS, M1 Max, 64 GB | 9.62-9.79 s vs rsync 8.39-8.56 (**~0.868**) | 8.54-8.56 s vs 8.10-8.14 (**~0.956**) |
| mars → orion | Linux, 7900X, 30 GB | 9.47 s vs rsync 8.18 (**0.864**) | 8.22 s vs 7.89 (**0.960**) |

Two entirely different senders — different OS, CPU architecture, memory, and
even a better link (mars→orion measures 116.2 MB/s and 0.73 ms RTT against the
Mac's ~113 MB/s and 1.7 ms) — produce the same deficit to within noise. **The
gap belongs to the receiver.** 4.66 assumed this; it had never been controlled
for, since every prior measurement used the Mac as sender.

## It decomposes into two parts, and only one is the disk

| | gap |
|---|---:|
| ext4 | ~1.2-1.29 s |
| tmpfs | ~0.33-0.43 s |
| **attributable to disk** | **~0.9 s** |
| **residual with no disk at all** | **~0.35 s** |

## Flush cadence is not the mechanism

`XSYNC_RECEIVER_FLUSH_CHUNKS` was added to test the hypothesis that flushing as
we go *contends* with the receive path rather than merely costing its own time.
`1` is today's per-chunk behaviour, `N` flushes every N chunks, `0` defers
everything to `LargeFileFinish`.

| cadence | durable |
|---|---:|
| 1 (per chunk, default) | 9.67 / 9.79 s |
| 8 | 9.67 / 9.80 s |
| 0 (defer to file end) | 9.78 / 9.82 s |

**No effect.** Deferring every flush to the end of an 847 MB file is worth
nothing, and is fractionally *slower*.

**Why**, and this is the useful part: dirty pages never accumulate in either
mode — 1.6 MB median during transfer at both cadences, peaking at 5.2 MB and
3.7 MB. ext4 mounts `data=ordered` with a 5-second journal commit, and ordered
mode requires data blocks written before each commit, so **the filesystem
flushes our data every five seconds whether we ask or not.** Our explicit
`fsync` cadence cannot matter because ext4 was never letting the pages sit.

This also explains the earlier raw-device result, where cadence was worth only
0.36 s: the same journal behaviour applies there.

The knob is kept, defaulted to `1` so behaviour is unchanged. It is a real
control for a filesystem that *does* defer — `data=writeback`, XFS, f2fs —
and for the low-powered-device heuristic that motivated it. On ext4 in ordered
mode there is nothing for it to win.

## Receiver memory is not the mechanism either

| | receiver RSS peak | median |
|---|---:|---:|
| rsync | 10.7 MB | 10.7 MB |
| xsync | 29.0 MB | 12.2 MB |

xsync's higher memory use is real but it is a *sender* phenomenon (298 MB on
congress-100k, 129-135 MB on large-file corpora). On the receiver the
difference is ~18 MB at peak on a 4 GB machine, which cannot cost a second.
Sender memory remains a separate concern for the Pi *as sender* (4.23).

## What is left

Ruled out by measurement: fsync cadence, write pattern, `sync_file_range`,
receiver memory, sender platform, and the link. What remains is ~0.9 s of disk
write sitting on the critical path, plus ~0.35 s that persists to RAM and has
no explanation yet.
