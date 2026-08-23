# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `deep-small-100k`
- Corpus manifest: `76a260da38f88fea67ea03accdb95904e7c61d69a129de2abf830c9f75533c01`
- Host/OS: `MacBookPro` / `macos` / `Darwin 25.6.0`
- Hardware: `Apple M1 Max`
- Filesystem: `apfs`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 907095 entries/s |
| Scan rate MAD | 24786 entries/s |
| Syscall-sensitive scan median | 0.220484 s |
| Destination index median | 0.011685 s |
| Planner median | 0.024548 s |
| Peak RSS | 93437952 bytes |
| Queue high-water | 1024 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100000 | 844817 | 0.236738 | 0.011685 | 0.024259 | 91340800 | 1024 |
| 1 | 100000 | 931881 | 0.214620 | 0.011607 | 0.024835 | 91226112 | 1024 |
| 2 | 100000 | 870881 | 0.229653 | 0.011878 | 0.024532 | 93437952 | 1024 |
| 3 | 100000 | 908271 | 0.220198 | 0.012815 | 0.028539 | 88752128 | 1024 |
| 4 | 100000 | 907095 | 0.220484 | 0.011684 | 0.024548 | 85966848 | 1024 |

## Qualifications

- First repetition is first-pass; later repetitions are warm-cache. No cache eviction was claimed.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
