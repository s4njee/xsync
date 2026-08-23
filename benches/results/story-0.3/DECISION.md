# Story 0.3 — scanner, planner, and memory decision

Date: 2026-08-23

## Decision

- Keep the existing portable `ignore::WalkParallel` scanner. Do not adopt the timeboxed macOS
  `getattrlistbulk+openat` prototype.
- Use **512 MiB peak RSS** as the provisional 1M-entry scanner/planner budget. The measured ext4
  run peaked at 478,515,200 bytes (456.3 MiB, 89.1% of budget), so Story 0.3 does not trigger its
  conditional rule making the current destination `HashMap` a protocol blocker.
- Keep Story 2.2b in the v1 backlog. The current planner still retains two complete scan vectors and
  a destination `HashMap`, the 1M result has little headroom, and the bounded queue reached its full
  1,024-entry capacity. The evidence supports a future explicit budget/spill boundary; it does not
  support calling the current memory shape corpus-independent.
- Make no general macOS-versus-Linux or filesystem performance claim. APFS was measured on one
  Apple M1 Max host; ext4 and tmpfs were measured on one AMD Ryzen 9 7900X Linux host.

## Method

`xsync-engine-bench` runs every repetition in a fresh release-build process. Each worker scans the
same independently manifested corpus twice, builds the current destination `HashMap`, and runs the
metadata planner. Linux peak RSS comes from `/proc/self/status` `VmHWM`; macOS peak RSS comes from
`/usr/bin/time -l`. The report preserves all five repetitions and separately records:

- entries/s across the two scanner passes;
- combined syscall-sensitive scan wall time;
- destination-index construction time;
- planner classification time;
- process peak RSS; and
- producer-observed bounded-channel high-water.

The first repetition is a truthful first-pass observation. Later repetitions are warm-cache; no
cache eviction was attempted or claimed. Every reported corpus uses seed 0 and the Story 0.1
independent BLAKE3 manifest.

## Results

| Host | Filesystem | Corpus | Scan median | Scan MAD | Syscall phase | Index | Planner | Peak RSS | Queue HWM |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|
| MacBookPro, M1 Max | APFS | flat-small 100k | 637,230/s | 1,223/s | 0.313858 s | 0.007905 s | 0.017016 s | 281,264,128 B | 1024/1024 |
| MacBookPro, M1 Max | APFS | deep-small 100k | 907,095/s | 24,786/s | 0.220484 s | 0.011685 s | 0.024548 s | 93,437,952 B | 1024/1024 |
| mars, Ryzen 9 7900X | ext4 | flat-small 100k | 1,132,801/s | 47,802/s | 0.176553 s | 0.009333 s | 0.011624 s | 54,542,336 B | 1022/1024 |
| mars, Ryzen 9 7900X | ext4 | deep-small 100k | 2,897,848/s | 33,622/s | 0.069017 s | 0.020308 s | 0.036912 s | 75,853,824 B | 1024/1024 |
| mars, Ryzen 9 7900X | tmpfs | flat-small 100k | 982,292/s | 18,409/s | 0.203605 s | 0.013950 s | 0.016058 s | 54,243,328 B | 1024/1024 |
| mars, Ryzen 9 7900X | tmpfs | deep-small 100k | 2,999,820/s | 84,781/s | 0.066671 s | 0.021803 s | 0.038371 s | 72,134,656 B | 1024/1024 |
| mars, Ryzen 9 7900X | ext4 | flat-small 1M | 1,232,865/s | 60,595/s | 1.622238 s | 0.201984 s | 0.239144 s | 478,515,200 B | 1024/1024 |

All scan MAD/median ratios are below the 15% unverified threshold. Values are host/filesystem
observations, not universal throughput promises.

## macOS `getattrlistbulk` timebox

The existing `f2` prototype was run for five repetitions against the exact APFS deep-small-100k
source used by xsync. Its warm median was 151.9 ms, or about 658k entries/s.
`xsync-engine-bench` measured the current portable scanner at about 907k entries/s on the same tree,
so the prototype achieved only **0.73x** the portable rate (portable was about 1.38x faster).

The prototype also fails xsync's semantic adoption gate: it converts names with
`String(cString:)`, does not preserve raw non-UTF-8 path bytes, and does not return the complete mode
metadata required by the wire manifest. It is therefore rejected on both performance and
correctness-contract grounds. Story 2.1b must establish reversible paths before any future platform
backend can be considered.

## Linux filesystem qualification

`mars.local` was verified rather than assumed:

- `/home/sanjee` is ext4 on `/dev/nvme1n1p2`;
- `/tmp` is a 15.2 GiB RAM-backed tmpfs.

Both flat and deep 100k shapes were recorded on both filesystems. This satisfies the two-filesystem
matrix for this host, but one Linux machine is insufficient for a cross-platform scanner claim.

## Artifacts

The adjacent `*.json` files are authoritative versioned reports containing all repetitions. The
matching generated `*.md` files are their human-readable renderings.
