# orion: the deficit is the disk write, measured on two devices

`large1gb` (847 MB, 4 files), Mac → orion over the router1 switch, three
interleaved repetitions, measured to durability, content SHA-256 verified.

**One binary throughout: `48b2adab64f1`, clean** (no uncommitted work). This
matters — see the last section.

| device | raw seq write | disk time for 847 MB | rsync | xsync | ratio | xsync − rsync |
|---|---:|---:|---:|---:|---:|---:|
| internal NVMe, PCIe Gen3 | 765 MB/s | 1.11 s | 8.47 s | 9.76 s | 0.868 | **1.29 s** |
| USB 3.1 NVMe (UAS, 5 Gbps) | 340 MB/s | 2.49 s | 8.93 s | 11.24 s | 0.795 | **2.31 s** |

## What this establishes

**xsync's deficit is the disk write, essentially in full.** Across two devices
differing 2.25x in write speed, the gap to rsync tracks the time it takes to
write the corpus: 1.29 s against 1.11 s of disk on the internal drive, 2.31 s
against 2.49 s on the USB one. Nothing else in the system scales that way.

**rsync barely notices the device.** It moves 0.46 s between a 765 MB/s drive
and a 340 MB/s one; xsync moves 1.48 s, 3.2x more. rsync defers writeback and
lets the kernel drain after it exits, so the destination's speed is largely
invisible to it. xsync flushes as it goes and pays the device rate directly.

This is now measured three independent ways: by removing the disk (tmpfs,
0.956), by making the disk faster (PCIe Gen3, 0.806 → 0.868), and by making it
slower (USB, 0.795). All three agree.

## A negative result worth recording

The internal-drive figure here (9.76 s) is **indistinguishable from the same
cell measured with `LargeChunkPool` present** (9.74 s, binary
`943b8ec49bc2-dirty`). That pool is wired into `run_sink`, bounded by the
sender's window so acknowledgements keep flowing, and is a correct answer to
the deadlock that killed the first attempt at R1 — but on this path it is not
yet producing overlap. The writer thread exists; the disk time is still on the
critical path.

So R1's remaining question is not the design of the pool. It is why a receiver
that already has a writer thread still pays the full serialized write.

## Ruled out, with numbers

- **fsync cadence** — at Gen3 the whole write costs 1.12 s with one flush and
  1.48 s flushing every 8 MB. Batching is worth at most 0.36 s, and was
  separately tried in code and measured at zero.
- **write pattern** — sparse-and-reopen matched sequential append (414.2 vs
  413.7 MB/s).
- **`sync_file_range` hint** — implemented and reverted at 0.7%, inside the
  noise, and it never fired on the measured path because the receiver's writer
  flushes every chunk.
