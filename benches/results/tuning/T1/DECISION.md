# T1 tuning status — per-file syscall gap

Date: 2026-08-24

## Current result

The first isolated optimization changed `SourceReader` so its read buffer is bounded by the
scanned file size, up to 64 KiB, instead of allocating 64 KiB for every file. The helper has unit
coverage for empty, small, bounded, and oversized inputs.

The five-repetition real-corpus run is in `buffer-sized/`:

| Method | Median wall | Median CPU | Peak RSS | Paired ratio vs rsync-a | Oracle |
|---|---:|---:|---:|---:|---|
| rsync-a | 3.123 s | 5.167 s | 54,132,736 B | baseline | 5/5 pass |
| xsync (buffer-sized) | 6.724 s | 28.752 s | 41,664,512 B | 0.471 | 5/5 pass |

Corpus identity: `congress-10k`, 22,568 items, 96,542,108 logical bytes,
`f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`. Both rows were below the
15% MAD/median noise policy except for no applicable comparison issue; the result is valid but does
not meet T1.3's target.

## T1.1 — syscall attribution

`dtruss -c -f` was attempted on a disposable `congress-1k` copy and failed with:
`dtrace: failed to initialize dtrace: Operation not permitted`. The available `fs_usage` fallback
also requires root. macOS System Integrity Protection is therefore the blocker for the required
per-syscall histogram. No syscall counts are fabricated from the aggregate benchmark timings.

A second attempt after Full Disk Access was enabled produced the same SIP diagnostic:
`dtrace: system integrity protection is on, some features will not be available`, followed by
`dtrace: failed to initialize dtrace: Operation not permitted`. `fs_usage` reported
`must be run as root`, while `sudo` was unavailable in the Codex environment. Full Disk Access
is not equivalent to the root entitlement required by these tracing tools.

The user-run script was then executed with `sudo` against the congress-10k source. Both tools
completed successfully and both disposable destinations contain 11,280 regular files. The
captured files are under `syscall-trace-20260824-094017/`. Despite the privileged invocation,
both `xsync-dtruss.txt` and `rsync-dtruss.txt` contain only the SIP warning and an empty `CALL /
COUNT` table. The run therefore verifies that the transfer path works, but still provides no
syscall counts for T1.1.

## T1.2 — known per-file waste

The buffer-size candidate is implemented and measured, but the change is not sufficient to claim
completion. An independent five-repetition APFS clone spike measured the crossover for staged
file clones: clone/copy speedup was 0.502x at 4 MiB, 0.863x at 8 MiB, 1.130x at 12 MiB, and
1.448x at 16 MiB. The local worker now skips the staged clone attempt below an empirically chosen
12 MiB threshold (`FILE_CLONE_MIN_BYTES`), avoiding the slower path for the many small files in
the real corpus. The threshold change is isolated from the buffer change and is covered by the
workspace tests.

The deterministic temporary-path hash candidate has now been implemented and independently
measured without changing the sink naming contract. `Sink` shares
a cache across cloned worker sinks, so repeated temporary-path lookups reuse the relative-path
hash. The paired five-repetition runs are `hash-baseline/` and `hash-cached/`:

| Variant | xsync median wall | xsync median CPU | Paired ratio vs rsync-a | Oracle |
|---|---:|---:|---:|---|
| uncached hash | 2.992617 s | 10.013016 s | 1.077x | 5/5 pass |
| cached hash | 3.070423 s | 10.250955 s | 1.020x | 5/5 pass |

The cache is within the harness noise policy and does not demonstrate a performance improvement;
the existing filename contract and correctness are preserved.

## T1.3 — syscall budget target

The current `congress-10k` result is 0.471x paired wall speed and roughly 5.6x the rsync CPU time,
so it misses both the required wall ratio (0.9) and system-time budget (1.5x). The 100k confirmation
cell was not run because the 10k prerequisite fails; it would not be valid to present it as a pass.
The remaining work is implementation work, not a user-provided environment blocker.

## Plain-English summary

We finished the measurable part of T1.2 and verified every destination in the new hash comparison.
T1.1 is still blocked by macOS tracing permissions. T1.3 is still blocked because xsync has not
reached the required speed and system-CPU targets, so the 100k confirmation is intentionally not
claimed.
