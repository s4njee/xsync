# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `flat-small-1m`
- Corpus manifest: `2186852abece131c447eae1809c1ca8d350837f61ba191b2506e329ac51a654e`
- Host/OS: `mars` / `linux` / `Linux 7.1.6-arch1-1`
- Hardware: `AMD Ryzen 9 7900X 12-Core Processor`
- Filesystem: `ext4`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 1232865 entries/s |
| Scan rate MAD | 60595 entries/s |
| Syscall-sensitive scan median | 1.622238 s |
| Destination index median | 0.201984 s |
| Planner median | 0.239144 s |
| Peak RSS | 478515200 bytes |
| Queue high-water | 1024 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 1000000 | 1303843 | 1.533927 | 0.203694 | 0.239144 | 448286720 | 1024 |
| 1 | 1000000 | 1281124 | 1.561129 | 0.200748 | 0.235024 | 478515200 | 1024 |
| 2 | 1000000 | 1232865 | 1.622238 | 0.201984 | 0.239804 | 477687808 | 1024 |
| 3 | 1000000 | 1172270 | 1.706092 | 0.201047 | 0.236297 | 468905984 | 1024 |
| 4 | 1000000 | 1138807 | 1.756224 | 0.204475 | 0.241334 | 459612160 | 1024 |

## Qualifications

- Explicit 1M scale gate on ext4; first repetition is first-pass and later repetitions are warm-cache.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
