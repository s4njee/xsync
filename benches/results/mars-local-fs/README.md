# congress-1m, local NVMe → SATA SSD: ext4 versus XFS

**The XFS deficit reported in the first version of this file was a bug in
xsync, not a property of XFS.** Fixing it takes xsync from 1.61x *slower* than
rsync on XFS to 1.07x *faster*, and makes the two destination filesystems
perform the same. The original analysis, which blamed the per-file publication
sequence and `rmapbt=1`, is retained at the bottom because both of its
hypotheses were tested and both were wrong.

- Host: mars (Ryzen 9 7900X, 24 threads, 30 GB RAM)
- Source: `~/congress-1m` on the internal NVMe (ext4), 1,318,771 files, 13 GB
- Destinations: Inland 1 TB SATA SSD, `sda1` ext4 → `/mnt/ext4`,
  `sda2` XFS → `/mnt/xfs` (`crc=1, rmapbt=1, reflink=1`, `inode64,logbufs=8,logbsize=32k`)
- Two interleaved repetitions, fresh destination directory per run so removal is
  never on the timed path, `sync` inside the timer, every run verified at
  1,318,771 landed files.
- `OLD` = `53a9a25f`. `FIXED` = the same tree plus the reflink-probe fix below.

| destination | xsync OLD | xsync FIXED | rsync -a |
|---|---|---|---|
| XFS | 155.06 / 163.11 | **90.07 / 84.48** | 96.40 / 113.44 |
| ext4 | 88.81 / 78.72 | **88.12 / 80.43** | 97.46 / 93.79 |

Taking the worse xsync run against the better rsync run in each pair:

| destination | before | after |
|---|---|---|
| XFS | xsync **1.61x slower** | xsync **1.07x faster** |
| ext4 | xsync 1.06x faster | xsync **1.06x faster** (unchanged) |

ext4 is the control: the fix must not affect it, because ext4 has no reflink and
so never entered the broken path. It does not — 88.81→88.12 and 78.72→80.43 are
both inside run-to-run noise. After the fix the destination filesystem barely
matters to xsync (90.07 XFS against 88.12 ext4), which is the expected result
and was the first sign that the original "filesystems interact with how each
tool writes" framing was describing a bug rather than a filesystem.

## The bug

`clone::supports_reflink` probed by creating a file **inside the destination**
and cloning it to a second file **inside the destination**. That asks "can this
filesystem reflink to itself?" — but the operation about to be performed is a
clone *from the source into the destination*. On a cross-device copy the two
questions have different answers, and the probe always returned the wrong one.

XFS here has `reflink=1` and ext4 has no reflink at all, so only the XFS arm
entered the path. xsync concluded cloning was available and ran
`cp -a --reflink=always` across the tree; because source (NVMe) and destination
(SATA SSD) are different devices, every file failed `EXDEV`. xsync then removed
the entire staged tree and performed the ordinary copy anyway. All of that work
is pure overhead, and it scales with file count — which is why 1.3 million files
made it so visible.

`strace -f` shows both ioctls next to each other, one line apart:

```
40703 ioctl(4, FICLONE, 3) = 0                                  # probe, dest → dest
40704 ioctl(4, FICLONE, 3) = -1 EXDEV (Invalid cross-device link) # real, src → dest
```

A syscall census over a 40,389-file subset, same tool and corpus, destination
filesystem the only variable:

| syscall | xs → ext4 | xs → XFS (old) | xs → XFS (fixed) |
|---|---|---|---|
| ioctl | 0 | 40,391 (40,390 `EXDEV`) | 2 |
| unlinkat | 4 | 78,160 | 4 |
| mkdirat | 0 | 37,768 | 0 |
| fchownat | 0 | 37,768 | 0 |
| write | 83,771 | 285,712 | 83,772 |
| getdents64 | 75,565 | 226,683 | 75,565 |

The fixed XFS profile is identical to the ext4 profile. The whole divergence was
the failed clone and its cleanup.

## The fix

Probe by cloning a **real source file** into the destination, which tests the
operation actually about to run. The probe only ever reads the source, so it is
safe against read-only and immutable source trees.

Comparing `st_dev` would be cheaper and would fix this case, but it is wrong:
btrfs supports reflink *across subvolumes*, which have different device IDs, so
an `st_dev` check would silently disable cloning for btrfs users. Cloning a real
source file is correct for cross-device, same-device, and cross-subvolume alike.

Verified that APFS same-volume directory cloning still fires afterwards
(`directory_clones: 1`, `byte_copies: 0`), since that fast path is where the
local-copy speedup comes from.

## Superseded: the original analysis

The first version of this file concluded that xsync's per-file publication
sequence — stage to a temp file, verify, apply metadata, rename — was too
expensive against XFS with `rmapbt=1`, and proposed reformatting without
`rmapbt` as the next test. Both halves were wrong, and the evidence that killed
them is worth keeping.

**The publication sequence is not expensive.** A microbenchmark replaying
xsync's exact per-file syscall sequence, 200,000 files at 10 KB, one variant per
suspected factor:

| variant | ext4, 1 thread |
|---|---|
| xsync sequence (64-char temp name, path metadata, rename) | 12.00 s |
| rsync-style (short temp name, fd metadata, rename) | 12.04 s |
| no metadata at all | 11.91 s |
| direct write, no temp file, no rename | 11.11 s |

Every publication variant is within noise of every other. The 64-character
BLAKE3 temp name costs nothing measurable; path-based `chmod`+`utimensat`
versus fd-based costs nothing; removing metadata entirely saves ~1%. Only
abandoning stage-and-rename altogether buys ~7%, and that trades away atomic
publication.

**The `--local-workers` sweep was also a dead end**, though its numbers stand:
24 → 133.06 s, 8 → 133.59 s, 4 → 140.27 s, 1 → 196.31 s on the old binary. More
parallelism helped because it hid more of the wasted clone work, not because
XFS rewards concurrency.

`rmapbt` was never tested and no longer needs to be. No partition was
reformatted.

## Method notes

Three earlier measurement attempts were discarded rather than reported:

1. The microbenchmark's temp-name generator derived all 64 hex characters from
   four bits of the file id, so 24 threads collided on 16 distinct names and
   raced on `rename`.
2. It called `sync()`, which is system-wide, so each cell's timer flushed the
   previous cell's `rm -rf` and the other filesystem's dirty pages. Switched to
   `syncfs()` on the target filesystem with teardown and settling moved before
   the timer.
3. The first full corpus matrix used `bc`, which is not installed on mars; all
   eight transfers completed correctly but every timing was lost.

Absolute times here run slower than the superseded table (ext4 xsync 88 s here
against 76 s there) because page-cache warmth differed between sessions. Arms
are interleaved within a session, so the ratios are the comparable quantity, not
the absolute seconds.
