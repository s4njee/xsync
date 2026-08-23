# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `deep-small-100k`
- Corpus manifest: `76a260da38f88fea67ea03accdb95904e7c61d69a129de2abf830c9f75533c01`
- Host/OS: `mars` / `linux` / `Linux 7.1.6-arch1-1`
- Hardware: `AMD Ryzen 9 7900X 12-Core Processor`
- Filesystem: `tmpfs`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 2999820 entries/s |
| Scan rate MAD | 84781 entries/s |
| Syscall-sensitive scan median | 0.066671 s |
| Destination index median | 0.021803 s |
| Planner median | 0.038371 s |
| Peak RSS | 72134656 bytes |
| Queue high-water | 1024 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100000 | 2866048 | 0.069783 | 0.021803 | 0.032675 | 71098368 | 1024 |
| 1 | 100000 | 3069042 | 0.065167 | 0.021905 | 0.032928 | 70877184 | 291 |
| 2 | 100000 | 2999820 | 0.066671 | 0.021763 | 0.045676 | 70316032 | 191 |
| 3 | 100000 | 2909547 | 0.068739 | 0.021770 | 0.045191 | 72134656 | 674 |
| 4 | 100000 | 3084601 | 0.064838 | 0.023306 | 0.038371 | 71749632 | 1024 |

## Qualifications

- First repetition is first-pass; later repetitions are warm-cache. No cache eviction was claimed.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
