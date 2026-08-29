# Hashing cost, congress-100k results, and a 6.3x macOS finding

Date: 2026-08-28.

## 1. "Hash less" — measured, and there is nothing to win

The `perf` profile attributed **17% of CPU samples** to BLAKE3, which raised the
question of whether a cheaper hash (or no hash) would help. It does not.

A temporary toggle skipped the streaming BLAKE3 in `SourceReader` and the
corresponding check in `Sink::write_file_with_retry`, giving the upper bound on
any hashing optimization — not a faster hash, but *no hash at all*:

| host / corpus | with hashing | hash skipped | difference |
|---|---:|---:|---|
| APFS, congress-10k (`--no-directory-clone`), 7 reps | 2.515 s | 2.513 s | **none** |
| ext4, congress-10k, 7 reps | 0.195 s | 0.195 s | **none** |

**17% of samples is not 17% of time.** Hashing runs on worker threads that are
otherwise blocked on I/O, so it consumes cycles without extending the critical
path. Removing it entirely saves nothing measurable.

This also settles the SHA-256 question. Measured throughput:

| | BLAKE3 | hardware SHA-256 (openssl) |
|---|---:|---:|
| Ryzen 9 7900X (AVX-512, SHA-NI) | **6.59 GB/s** | 2.66 GB/s |
| Apple M1 Max (NEON, FEAT_SHA256) | 1.67 GB/s | **2.40 GB/s** |

SHA-256 is 1.44x faster on Apple Silicon and 2.5x *slower* on x86 — but since the
hash is off the critical path on both, the point is moot. Switching would also
break the wire protocol and forfeit BLAKE3's tree structure, which is what makes
chunk-level resume possible at all. Adding blake3's explicit `neon` feature was
also tested and changed nothing (1.69 vs 1.67 GB/s); NEON is already active.

**Conclusion: keep BLAKE3, keep hashing. Neither is costing wall-clock time.**

## 2. congress-100k

**Linux, ext4/NVMe, 24 cores.** 92,911 files, 32,628 directories, 1.4 GB
(sessions 110+111 hardlinked; mars's tree has no single 100k subtree). Five
repetitions, median:

| | wall | MAD | sys | user |
|---|---:|---:|---:|---:|
| `rsync -a` | 1.925 s | 0.5% | 2.522 s | 1.010 s |
| **xsync** | **1.012 s** | 0.3% | 3.860 s | 2.544 s |

**xsync is 1.90x faster**, improving on the 1.67x measured at 10k — the advantage
grows with scale. It spends more CPU on both axes and wins on wall clock through
parallelism.

**macOS, APFS, 10 cores.** 109,615 files, 135,466 items, 584 MB. Five
repetitions through the release harness, both rows comparable:

| | wall | MAD | CPU | peak RSS |
|---|---:|---:|---:|---:|
| `rsync -a` | 24.024 s | 2.0% | 31.397 s | 44,285,952 |
| xsync | 28.310 s | 1.3% | 29.632 s | **190,840,832** |

Ratio **0.842**. Two things stand out: xsync uses *less* CPU than rsync here yet
takes longer, and its peak RSS is **4.3x** rsync's. At the 1.3M-file corpus that
memory figure needs its own look before `congress-1m` is attempted.

## 3. Why macOS is slow: `cp -c -R` is not a tree clone

At this scale xsync publishes the whole tree as a single directory clone —
`directory_clones: 1`, `file_clones: 0`, `byte_copies: 0`, 109,615 files with zero
bytes copied. That should be nearly instant. It takes 28 s.

The cause is that `clone::platform_clone_file`/`platform_clone_directory` shell out
to `/bin/cp -c -p -R`, and **`cp -c` performs a per-file `COPYFILE_CLONE`, not a
single tree-level `clonefile()`**. Measured directly on the same 109,615-file tree:

| method | time |
|---|---:|
| `clonefile()` on the tree root (one syscall) | **3.766 s** |
| `cp -c -p -R` (what xsync uses) | 23.610 s |

**6.3x, for byte-identical output** — both produced 109,615 files. This is exactly
the distinction f2 §1 measured as 22x versus 2.70x, and xsync is on the wrong side
of it.

If the clone took 3.77 s instead of 23.61 s, xsync's 28.31 s would fall to roughly
8.5 s and the ratio against rsync's 24.02 s would go from **0.842 to about 2.8x
faster**.

**The obstacle is `unsafe`.** `clonefile(2)` needs libc FFI, and the workspace sets
`unsafe_code = "deny"`. Unlike the io_uring case — where the measured prize was 4%
and the answer was clearly no — this is 6.3x on the dominant macOS path, so it is
a real decision rather than an obvious refusal. The same question applies to
`FICLONE` on Linux, which would remove a process spawn per clone.

## Method note

Both toggles used here (`XSYNC_SKIP_HASH`, and the earlier dispatch toggle) were
temporary, applied to a single binary so that only one variable changed, and
removed afterwards. The tree is clean; 172 tests and strict clippy pass.
