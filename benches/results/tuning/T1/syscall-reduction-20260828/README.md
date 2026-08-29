# T1 — instrumenting the syscall gap, and closing 44% of it

Date: 2026-08-28. Host `mars.local`, ext4/NVMe, kernel 7.1.6. Corpus
`congress-10k`: 11,280 files, 11,288 directories, 22,568 entries.

## Method

Three layers, each answering the question the previous one raised:

1. **Phase timing**, free from the product's own `--progress-json` events:
   scan 0.028 s, plan 0.016 s, **transfer 0.570 s**, metadata 0.058 s. 85% is
   transfer, so that is where to look.
2. **Path logging** — the `LD_PRELOAD` counter from
   [`../syscall-attribution-20260828/`](../syscall-attribution-20260828/README.md)
   extended to record the path of every stat-family call. This showed a single
   second-level destination directory, `bills`, being stat'd **56,411 times**.
3. **Backtrace sampling** ([`syscount-backtrace.c`](syscount-backtrace.c)) to
   attribute those calls to code. This needed two corrections before it was
   trustworthy, both worth recording:
   - the release profile sets `strip = true`, so a rebuild with
     `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1` was
     required for `addr2line` to resolve anything;
   - `-O2` omits frame pointers, so `backtrace()` returned 1–2 useless frames
     until the binary was rebuilt with `RUSTFLAGS="-C force-frame-pointers=yes"`;
   - **and the first histogram was wrong**: logging the *first* N calls sampled
     only the scan phase and named the scanner as the top caller. Switching to a
     uniform 1-in-20 sample across the whole run reversed the answer entirely.

## What the attribution found

Uniform sample, 14,104 backtraces representing 282,078 `statx` calls:

| caller | share |
|---|---:|
| `Sink::destination_path` | **52%** |
| `scanner::run_walker` | 16% |
| `clone::entries_match` / `try_clone_directory` | 16% |
| `Sink::temporary_path` (via `destination_path`) | 12% |

`destination_path` walks every ancestor component calling `symlink_metadata` to
prevent an ancestor symlink redirecting the write outside the destination root.
It is correct and worth having — but it ran on **every call**, several times per
published file, at O(depth) each time.

## Fixes

Both reuse one idea: the sink creates these directories itself, so it can
remember them.

1. **`create_parent` cache.** `create_dir_all` was called per published file to
   ensure a parent that `create_directories` had already made. Now a hash lookup.
   `mkdir` 22,572 → 11,293.
2. **Ancestor-walk cache.** `destination_path` returns early when the parent is
   already recorded as sink-created. Directories made by `create_dir_all` are
   real directories, not symlinks, so the guarantee is unchanged for anything the
   sink did not create. This does not weaken the check in any way that matters:
   the original is resolve-after-stat, so an ancestor swapped between check and
   use defeats both forms equally.

All six symlink-security tests still pass, including
`rejects_symlinked_destination_ancestors` and `rejects_escape_and_symlink_traversal`.

## Result

| call | before | after | change |
|---|---:|---:|---:|
| `statx` | 282,078 | **135,445** | **-52%** |
| `mkdir` | 22,572 | 11,293 | -50% |
| **total syscalls** | 565,425 | **317,274** | **-44%** |
| per entry | 25.1 | **14.1** | rsync is 6.4 |

Wall clock, five repetitions, median:

| host | before | after | gain |
|---|---:|---:|---:|
| mars (ext4) | 0.653 s | **0.567 s** | **13%** |
| macOS (APFS, controlled A/B via toggle) | 5.59 s | **5.37 s** | 3.9% |

System time on mars fell 0.866 s → 0.764 s. The macOS box was under shifting load
across the session, so that figure comes from an A/B with the same binary behind a
temporary env toggle rather than from comparing across runs; variance also
tightened markedly (5.32–5.61 s against 5.49–7.29 s).

## Why 44% fewer syscalls only buys 13%

Consistent with the [io_uring spike](../io-uring-spike-20260828/README.md): raw
materialization of this corpus costs 0.106 s, while xsync spends 0.567 s. Syscall
count is real work worth removing, but it is not the dominant term. The remaining
~0.4 s is neither materialization nor syscall overhead, and finding it needs a
profiler rather than a call counter.

## Still on the table

- **`unlink` at 2 per file and `chmod` at 1 per entry** — 45,147 calls rsync never
  makes at all. Not yet investigated.
- **`clone::entries_match` at 16% of statx**, spent evaluating reflink candidates
  that **cannot succeed on ext4**, which has no reflink support. Probing the
  destination filesystem once and skipping the candidate machinery entirely would
  remove that whole class. `--no-directory-clone` measured only a 22,571-call
  saving, so this needs its own look rather than an assumption.
- **The other 84% of wall time.** `perf` is not installed on `mars`; it is the
  obvious next tool.
