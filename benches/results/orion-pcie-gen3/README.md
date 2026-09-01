# orion: PCIe Gen2 → Gen3, and what it did not fix

## The change

The Pi 5 boots its NVMe at PCIe Gen2 by default. `orion` was running at
`5.0 GT/s x1` against a `max 8.0 GT/s`. Added `dtparam=pciex1_gen=3` to
`/boot/config.txt` (backup at `/boot/config.txt.pre-gen3-backup`) and rebooted.

- Link after reboot: **8.0 GT/s x1**, confirmed via
  `/sys/class/nvme/nvme0/device/current_link_speed`.
- Sequential write: **414 → 765 MB/s** (1.85x).
- No NVMe errors in `dmesg`; filesystem clean; content verified by independent
  SHA-256 across every subsequent run. **Gen3 is stable on this drive.**

Gen3 is officially uncertified on the Pi 5 and some drives are unstable on it.
This one is not, but that is a property of this drive, not of the platform.

## Effect on the gap (`large1gb`, 847 MB, measured to durability)

| | rsync | xsync | ratio |
|---|---:|---:|---:|
| Gen2 | 8.68 s | 10.77 s | 0.806 |
| **Gen3** | **8.39 s** | **9.74 s** | **0.861** |

xsync gained **1.03 s**, rsync **0.29 s** — xsync benefits ~3.5x more from a
faster disk, which is 4.66's diagnosis confirmed by a hardware change rather
than by code. The ratio improves but does not close: tmpfs is 0.956.

## What the remaining gap is *not*

**It is not fsync cadence.** At Gen3, writing 847 MB in 8 MB chunks:

| fsync cadence | time | throughput |
|---|---:|---:|
| every 8 MB (what the receiver does) | 1.48 s | 572 MB/s |
| every 64 MB | 1.37 s | 620 MB/s |
| once at end | 1.12 s | 759 MB/s |

Batching the flush is worth at most **0.36 s**. This is the second time that
idea has been measured and found small: batching alone was tried in code and
changed nothing, because the same bytes are flushed either way and fsync cost
tracks bytes rather than calls.

**It is not the write pattern.** Sparse-and-reopen measured identically to
sequential append (414.2 against 413.7 MB/s at Gen2).

**A `sync_file_range(SYNC_FILE_RANGE_WRITE)` hint was implemented and reverted.**
It measured 9.74 → 9.67 s, 0.7% and inside the noise, which does not justify a
second `unsafe` exemption in a workspace that documents having exactly one. It
also could not have helped much as wired: the push receiver's writer calls
`write_chunk_with_retry`, which flushes every chunk, so the hint never fired on
the path being measured.

## What it therefore is

The whole disk write is **1.12–1.48 s**, and xsync trails rsync by **1.28 s**.
The two are close enough that essentially none of the disk cost is being hidden,
even though `LargeChunkPool` is wired into `run_sink` and is bounded by the
sender's window so acknowledgements keep flowing.

So the remaining question is not *whether* to overlap but *why the existing
overlap is not taking effect*. That belongs to whoever owns the pool.
