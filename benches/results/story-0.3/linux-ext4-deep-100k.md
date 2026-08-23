# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `deep-small-100k`
- Corpus manifest: `76a260da38f88fea67ea03accdb95904e7c61d69a129de2abf830c9f75533c01`
- Host/OS: `mars` / `linux` / `Linux 7.1.6-arch1-1`
- Hardware: `AMD Ryzen 9 7900X 12-Core Processor`
- Filesystem: `ext4`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 2897848 entries/s |
| Scan rate MAD | 33622 entries/s |
| Syscall-sensitive scan median | 0.069017 s |
| Destination index median | 0.020308 s |
| Planner median | 0.036912 s |
| Peak RSS | 75853824 bytes |
| Queue high-water | 1024 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100000 | 2864227 | 0.069827 | 0.020308 | 0.036912 | 75853824 | 902 |
| 1 | 100000 | 2885682 | 0.069308 | 0.020494 | 0.045301 | 71020544 | 1024 |
| 2 | 100000 | 2897848 | 0.069017 | 0.020071 | 0.032528 | 72982528 | 1024 |
| 3 | 100000 | 2986378 | 0.066971 | 0.024059 | 0.036757 | 72019968 | 177 |
| 4 | 100000 | 2990160 | 0.066886 | 0.020288 | 0.044236 | 72847360 | 1024 |

## Qualifications

- First repetition is first-pass; later repetitions are warm-cache. No cache eviction was claimed.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
