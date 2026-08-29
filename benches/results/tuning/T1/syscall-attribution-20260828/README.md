# T1.1 — per-file syscall attribution (first successful capture)

Date: 2026-08-28. Host `mars.local`, Arch Linux, kernel 7.1.6, ext4 on NVMe.
Corpus `congress-10k` at `/home/sanjee/congress/data/100`: **11,280 files and
11,288 directories, 22,568 entries, 96,542,108 logical bytes**. Both tools copied
the identical tree to a fresh destination; both produced 11,280 files.

## Method, and why it is not strace

macOS SIP defeated `dtruss` even under a privileged capture, and `mars` has no
`strace`, `perf`, or `bpftrace`, with `sudo` requiring a password. Rather than
leave T1.1 blocked, calls were counted by interposing libc through `LD_PRELOAD`
([`syscount.c`](syscount.c), built with `gcc -shared -fPIC -O2`).

The counter was **validated before use** against a program making an exact known
number of calls (100 `open`, 100 `close`, 50 `stat`, 25 `lstat`) and reported
exactly those figures.

Limitations, stated because they bound the conclusions:

- It counts **libc entry points, not raw syscalls**. Anything reaching the kernel
  without going through libc is invisible. Notably `rename` shows zero for rsync,
  which is implausible — rsync likely uses `renameat2`, which is not interposed.
- `write` includes stdout and stderr, so it is not purely file I/O. xsync emits
  progress output where `rsync -a` is quiet, which inflates xsync's `write`.
- Counts are per process, summed across rsync's two processes.
- ext4, not APFS. The shape should carry over; the absolute costs will not.

## Result

| call | xsync | rsync -a | difference |
|---|---:|---:|---:|
| stat family (`statx`/`lstat`/`fstat`/`stat`) | **293,357** | 78,995 | +214,362 |
| `write` | 101,985 | 4,436 | +97,549 |
| `close` | 45,158 | 11,306 | +33,852 |
| `read` | 23,357 | 15,654 | +7,703 |
| `unlink` | 22,579 | 0 | +22,579 |
| `mkdir` | 22,572 | 11,288 | +11,284 |
| `chmod` | 22,568 | 0 | +22,568 |
| `utimensat`/`futimens` | 22,568 | 22,576 | -8 |
| `rename` | 11,280 | 0 (see limitations) | +11,280 |
| `fsync` | 1 | 0 | +1 |
| **total counted** | **565,425** | **144,284** | **3.9x** |

Per entry: **xsync ~25 calls, rsync ~6.4**. Per file: ~50 against ~12.8.

## Where the gap actually is

**Stat is over half of it.** 293,357 `statx` calls for 22,568 entries is **13 per
entry** — against rsync's 3.5. This single line accounts for 52% of the excess.
It is the first thing to attack, and it was not the thing anyone had guessed:
prior T1 work targeted read buffers, clone attempts, and path hashing.

**Operations rsync does not perform at all.** `unlink` at 2 per file and `chmod`
at 1 per entry are pure additions — 45,147 calls rsync never makes. rsync creates
its temporary with the correct mode and never unlinks a staging file that is not
there.

**`mkdir` runs twice per directory** (22,572 for 11,288 directories), consistent
with `create_dir_all` walking and retrying ancestors rather than creating known-new
leaves directly.

**`write` needs re-measuring with output suppressed** before its 23x ratio is
trusted; progress emission is mixed into it.

## What this says about the plan

- **T1.3's system-time target is reachable.** A 3.9x syscall ratio is consistent
  with the measured CPU gap, and the largest contributor is a single call class.
- **T1.4 (`io_uring`) stays gated, and this is why.** The gate asked whether the
  residual gap is dominated by *irreducible* syscalls. It is not: 13 stats per
  entry, an unlink rsync never makes, and a doubled mkdir are all avoidable work.
  Removing them is portable and needs no unsafe code. Batching submission would
  merely make unnecessary calls cheaper.
- **The next T1 story writes itself:** find where 13 stats per entry come from.
  Candidates are the scanner, the planner's destination probe, `create_dir_all`,
  and the sink's own checks — but that is a guess, and the point of this record is
  that guessing was what went wrong before.

## Correction to an earlier claim

A trace run earlier the same day showed 11,281 `cp` child processes — one per file
— and was nearly reported as a live defect. It was an artifact of the stale
`target/release/xsync` orphan described in
[`../T7/DECISION.md`](../T7/DECISION.md). The current binary spawns **two** `cp`
processes in total, and the 12 MiB `FILE_CLONE_MIN_BYTES` gate works as intended.
