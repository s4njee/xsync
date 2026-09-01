# congress-1m, local NVMe → SATA SSD: ext4 versus XFS

Same source, same machine, same binary, same run session — only the destination
filesystem differs. **xsync is 1.30x faster than rsync on ext4 and 1.58x slower
on XFS.**

- Host: mars (Ryzen 9 7900X, 24 threads, 30 GB RAM)
- Source: `~/congress-1m` on the internal NVMe (ext4), 1,318,771 files, 13 GB
- Destinations: Inland 1 TB SATA SSD, `sda1` ext4 → `/mnt/ext4`,
  `sda2` XFS → `/mnt/xfs`
- Binary `53a9a25fa5ce`; three interleaved repetitions; fresh destination
  directory per run so removal is never on the timed path; every run verified at
  1,318,771 landed files.

| destination | rsync -a (durable) | xsync (durable) | ratio |
|---|---|---|---:|
| ext4 | 99.25 / 100.11 / 92.85 → **99.25 s** | 72.09 / 76.47 / 78.04 → **76.47 s** | xsync **1.30x faster** |
| XFS | 91.55 / 83.75 / 86.66 → **86.66 s** | 143.88 / 137.35 / 134.65 → **137.35 s** | xsync **1.58x slower** |

Note rsync is *faster on XFS than on ext4* (86.66 against 99.25) while xsync is
*much slower* (137.35 against 76.47). The filesystem does not simply rank; it
interacts with how each tool writes.

## It is not worker count

xsync defaults to one local worker per core, 24 here, against rsync's single
thread — the obvious hypothesis. It is wrong:

| `--local-workers` | XFS durable |
|---:|---:|
| 24 (default) | **133.06 s** |
| 8 | 133.59 s |
| 4 | 140.27 s |
| 1 | 196.31 s |

More parallelism *helps* on XFS and the default is already optimal. Reducing it
makes things worse.

The single-worker row is the informative one. **rsync completes XFS in 86.66 s
single-threaded; xsync with one worker needs 196.31 s** — so xsync's per-file
work costs roughly **2.3x** rsync's on this filesystem, and its parallelism is
what claws that back to 137. On ext4 the same per-file work is cheap enough that
24 workers put xsync comfortably ahead.

## What to look at next

The per-file publication sequence is the suspect: xsync stages into a temporary
file, verifies, applies metadata, then renames. That is create + write + setattr
+ rename + unlink per entry, against 1.3 million entries.

This XFS is formatted with **`rmapbt=1`** (reverse-mapping btree) and `crc=1`,
and mounted `inode64,logbufs=8,logbsize=32k`. Reverse mapping updates metadata
on every extent allocation and free, which is exactly the pattern a
stage-and-rename publisher generates. Worth testing, in order:

1. An XFS filesystem built **without `rmapbt`** — the cheapest discriminator,
   and it needs only a reformat of the spare partition.
2. `logbsize=256k` — the log is the other obvious contention point.
3. Whether the staging rename can be avoided for files that do not already
   exist at the destination, which is the common case on an initial copy.

Until (1) is run, "XFS is slow for xsync" is not established — only "this XFS,
with reverse mapping enabled, is".
