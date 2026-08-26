# T3 tuning status — clone at the highest unchanged subtree

Date: 2026-08-24

## Current result

T3.1 now applies the existing staged directory-clone path to maximal directory subtrees that are
classified as wholly absent from an already populated local destination. A subtree that is only
partly absent or contains changed destination entries remains on the normal per-entry plan.

## Real-corpus smoke

Source: `congress-10k`, pinned manifest
`f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`.

The destination was seeded from the source with `bills/hconres` excluded. The current release
binary emitted:

```text
method=directory-clone path=bills/hconres bytes=3854086 physical_bytes=0
```

The independent full oracle then reported:

```text
passed=true
item_count=22568
logical_bytes=96542108
mismatch_count=0
hashed_file_count=11280
```

The disposable run directory was `/tmp/xsync-t3-real.7vyKAy`.

## 100k real-corpus smoke

The source `/Users/sanjee/projects/csearchv2/congress/data/118` was seeded into a disposable
destination while excluding `bills/hconres`. xsync then emitted one directory-clone event for
that absent subtree and completed in 2.55 seconds of wall time (`real 2.55`, `user 1.08`,
`sys 6.69`). The event reported 675 transferred files and 4,062,586 logical bytes, with zero
physical bytes and one directory clone.

The full independent oracle passed: 135,466 items, 583,940,018 logical bytes, 109,615 hashed
files, zero mismatches, and matching digest
`2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794`. The disposable run was
`/tmp/xsync-t3-100k/run-1787584121`.

## Cross-volume APFS smoke

The same seeded 100k source was copied to `/Volumes/XSYNC_BENCH/xsync-t3-100k-1787584449`.
xsync completed in 2.62 seconds (`real 2.62`, `user 1.10`, `sys 7.25`), emitted one
directory-clone event for `bills/hconres`, and reported 675 transferred files / 4,062,586
logical bytes. The full oracle passed with 135,466 items, matching digest, and zero mismatches.
Because both volumes are APFS and the directory clone succeeded, this does not exercise the
non-reflink fallback.

## Ext4 non-reflink smoke

The same seeded 100k source was transferred to `mars` at `192.168.1.119`, using ext4. Native
xsync completed in 2.06 seconds and reported `directory_clones=0`, `byte_copies=675`,
`transferred_files=675`, and `wire_bytes=857831`. The independent oracle ran on mars and passed
all 135,466 items with the pinned digest and zero mismatches. This confirms safe fallback to
ordinary byte copies when directory cloning is unavailable.

## Same-host clone/per-file point

To compare the same APFS source and seeded destination shape, a benchmark-only hidden CLI switch
(`--no-directory-clone`) disables only the directory fast path. Five repetitions for the
4,062,586-byte `bills/hconres` subtree produced:

| mode | median wall | MAD | final oracle |
|---|---:|---:|---|
| directory clone | 2.04 s | 0.02 s | pass |
| per-file fallback | 2.20 s | 0.13 s | pass |

The methods were run in separate five-repetition blocks without full order rotation or explicit
cache eviction, so this is directional evidence rather than a gate-able speedup claim. Both
final destinations matched the 135,466-item manifest with zero mismatches.

## Rotated crossover bracket

The first block was followed by a three-repetition block with per-file first and directory clone
second for every size, removing the consistent method-order bias. The hconres clone cell from the
first block was excluded because its freshly copied baseline caused unrelated files to be
rewritten; all rotated cells had zero failed entries.

| Missing subtree | Logical bytes | Files | Per-file median (MAD) | Clone median (MAD) | Faster in rotated block |
|---|---:|---:|---:|---:|---|
| `bills/hconres` | 4,062,586 | 675 | 2.22 s (0.12) | **2.06 s (0.01)** | clone |
| `bills/hjres` | 7,328,529 | 1,150 | **2.12 s (0.02)** | 2.40 s (0.01) | per-file |
| `bills/sres` | 17,729,567 | 4,710 | **2.72 s (0.10)** | 3.04 s (0.10) | per-file |
| `votes` | 115,680,459 | 3,864 | **2.62 s (0.01)** | 3.19 s (0.00) | per-file |
| `bills/hr` | 284,933,886 | 52,820 | **10.26 s (0.05)** | 14.67 s (0.15) | per-file |

This brackets the crossover between 4.06 MB and 7.33 MB for this APFS host, workload, and
implementation. It is not a universal filesystem threshold; the transfer process still includes
whole-tree scan/planning work, and the experiment does not replace the required phase-timing
report.

## 100k corpus pin

The source `/Users/sanjee/projects/csearchv2/congress/data/118` is available and was read-only
manifested with `target/release/xsync-bench`. It contains 109,615 files and 135,466 manifest
items, with 583,940,018 logical bytes and digest
`2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794`. The release benchmark
registry now pins this digest.

## Remaining work

The 100k correctness smoke, phase timings, rotated crossover bracket, and ext4 non-reflink
fallback evidence satisfy T3.1. The measured bracket is explicitly scoped to this APFS host and
workload; it is not a universal filesystem policy.

## Plain-English summary

The code now copies a whole missing directory in one operation when the rest of the destination is
already there, and the resulting tree is correct on APFS and ext4 fallback. On this Mac, cloning
won for a roughly 4 MB missing directory, while ordinary per-file copying won from roughly 7 MB
upward in the tested ladder. That is a useful local bracket, not a promise for every filesystem.
