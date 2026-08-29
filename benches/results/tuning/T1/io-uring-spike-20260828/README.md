# T1.4 — io_uring spike: measured, and not worth building

Date: 2026-08-28. Host `mars.local`, Arch Linux, kernel 7.1.6, 24 cores, ext4 on
NVMe, liburing 2.15, `io_uring_disabled=0`.

## Question

T1.4 was gated on whether xsync's residual per-file cost is dominated by
*irreducible* syscalls. If it is, batching submission through io_uring helps; if
the calls are avoidable, removing them is better and portable. Rather than argue
it, [`uring_spike.c`](uring_spike.c) measures the ceiling directly: how fast can
this machine materialize congress-shaped files with plain synchronous syscalls
versus io_uring batched submission?

The spike does exactly what xsync's sink does per file — create, write the
contents, close — at congress-10k's real scale (11,280 files, 8,559 byte average).
Two layouts: flat, and one directory per file, which is congress's actual shape
(11,288 directories for 11,280 files).

## Result

| layout | plain syscalls | io_uring (depth 256) | io_uring gain |
|---|---:|---:|---:|
| flat, one directory | 0.089 s (127,000 files/s) | 0.084–0.096 s | **none — slower in 2 of 3 runs** |
| one directory per file | 0.106 s (97,000 files/s) | 0.078–0.101 s | 1.15–1.35x |

Put against the real tools on the same host and corpus, three repetitions, median:

| | wall | sys | user |
|---|---:|---:|---:|
| **raw materialization floor (plain)** | **0.106 s** | — | — |
| **raw materialization floor (io_uring)** | **0.078 s** | — | — |
| `rsync -a` | 0.339 s | 0.396 s | 0.212 s |
| `xsync` | 0.653 s | 0.866 s | 0.500 s |

## Conclusion: do not build it

**io_uring's entire available win is 0.028 s** — the difference between the two
floors — against xsync's 0.653 s. That is **4% of wall time**, and only if xsync
were already at the materialization floor, which it is not. The realistic gain is
smaller still.

For that 4% the cost would be: unsafe code in a workspace that sets
`unsafe_code = "deny"`, a Linux-only path requiring a permanent portable fallback,
a subsystem disabled by policy in hardened environments, and a kernel-version
opcode matrix. **T1.4 is closed on evidence.**

Note the flat-layout row: io_uring was *slower* than plain syscalls in two of
three runs. Submission batching is not free, and at this scale the per-SQE
bookkeeping cancels the saved syscall entries. The gain in the deep layout comes
almost entirely from batching `mkdir`, not from the file operations.

## The more useful finding

**Raw file materialization is not the bottleneck for either tool.** Creating
11,280 directories and 11,280 files with contents costs **0.106 s**. xsync spends
0.653 s and rsync 0.339 s, so 84% and 69% of their respective wall times go
somewhere other than putting files on disk.

This does not invalidate T1 — xsync burns 0.866 s of system time against rsync's
0.396 s, so the 3.9x syscall-count gap from
[`../syscall-attribution-20260828/`](../syscall-attribution-20260828/README.md) is
real kernel work worth removing, and closing it should be worth roughly 2x. But it
does say the win comes from **making fewer calls**, not from making calls cheaper.
That is precisely what io_uring cannot do.

It also raises the next question, which is not a syscall question at all: what
occupies the ~0.55 s of xsync's wall time that is neither materialization nor
accounted for by the syscall gap? That is a profiling task, and guessing at it is
what produced the invalid experiment recorded in
[`../../T7/DECISION.md`](../../T7/DECISION.md).
