# T1 — `perf` profile: the clone fast path was 66% of wall time on ext4

Date: 2026-08-28. `mars.local`, ext4/NVMe, kernel 7.1.6, perf 7.1.8.
Corpus `congress-10k`. Built with `-C force-frame-pointers=yes` and unstripped so
`--call-graph fp` resolves; `perf_event_paranoid=2` limits sampling to user space,
which is where the answer turned out to be.

## What the profile showed

3,691 samples at 999 Hz. Two entries dominated, and neither was file I/O:

| | share | what it is |
|---|---:|---|
| `cp` subprocesses | **18.77%** | `cp -a --reflink=always`, the directory-clone attempt |
| crossbeam `recv<FileTask>` | **18.58%** | workers contending on the shared task queue |
| BLAKE3 (`compress_in_place_avx512` + `hash_many_avx512`) | ~6% | content hashing |
| `scanner::run_walker` + `ignore::walk` | ~5% | source enumeration |

The `cp` entry was the surprise. **ext4 has no reflink support**, so every clone
attempt is guaranteed to fail — but `cp -a --reflink=always` is spawned across the
tree and does substantial work before failing.

## Confirmation

Measured directly with the existing `--no-directory-clone` flag, five repetitions,
median:

| | wall |
|---|---:|
| default (clone attempts on) | 0.572 s |
| `--no-directory-clone` | **0.196 s** |

**The clone machinery cost 65.8% of total wall time on a filesystem where it can
never succeed.** For reference `rsync -a` copies the same corpus in 0.323 s, so
xsync was losing to rsync *entirely* because of a fast path that was not fast.

## Fix

`clone::supports_reflink()` probes the destination once per run by cloning a
single one-byte file — the same mechanism the real clone uses, so it cannot
disagree with it — and caches the result. Both the directory-clone pass and the
per-file clone attempt are gated on it. A probe failure reports "unsupported",
which costs only the fast path and never correctness.

## Result

| host / filesystem | before | after | vs `rsync -a` |
|---|---:|---:|---|
| mars, ext4 | 0.572 s | **0.192 s** | **1.670x faster** (rsync 0.321 s) |
| macOS, APFS | 5.37 s | 5.39 s | unchanged |

On ext4 this is a **2.98x speed-up**, turning a 1.93x loss against rsync into a
1.67x win. On APFS, where reflink genuinely works, the probe returns true and
behaviour is untouched: the run still reports `directory_clones: 1`,
`byte_copies: 0` — the whole tree cloned in one operation.

## Still open

- **Channel contention at 18.58%.** Workers spend nearly a fifth of the profile in
  `recv<FileTask>` on the shared queue. This is the same shared-queue structure
  whose partitioning was investigated in T7 — where the experiment was invalid, so
  the question remains genuinely open. It should be re-tested now, with the
  staleness guard in place, on a corpus with directory fanout.
- **`unlink` at 2/file and `chmod` at 1/entry**, 45,147 calls rsync never makes.
- Whether `cp --reflink` should be replaced by the `FICLONE` ioctl on Linux, which
  would avoid a process spawn per clone. That needs `unsafe`, which the workspace
  denies, so it is a deliberate decision rather than an obvious win.
