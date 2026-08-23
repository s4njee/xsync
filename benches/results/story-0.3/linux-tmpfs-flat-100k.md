# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `flat-small-100k`
- Corpus manifest: `19e0b6ec9c89a540dbd793ef72f5bfdcffd941e4cad5da55d873b63ec24173f3`
- Host/OS: `mars` / `linux` / `Linux 7.1.6-arch1-1`
- Hardware: `AMD Ryzen 9 7900X 12-Core Processor`
- Filesystem: `tmpfs`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 982292 entries/s |
| Scan rate MAD | 18409 entries/s |
| Syscall-sensitive scan median | 0.203605 s |
| Destination index median | 0.013950 s |
| Planner median | 0.016058 s |
| Peak RSS | 54243328 bytes |
| Queue high-water | 1024 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100000 | 974821 | 0.205166 | 0.013950 | 0.015249 | 51560448 | 1024 |
| 1 | 100000 | 963883 | 0.207494 | 0.015948 | 0.016058 | 50663424 | 1024 |
| 2 | 100000 | 982292 | 0.203605 | 0.011738 | 0.017682 | 48054272 | 1024 |
| 3 | 100000 | 1055451 | 0.189492 | 0.011318 | 0.013472 | 51830784 | 1024 |
| 4 | 100000 | 1152592 | 0.173522 | 0.015988 | 0.019224 | 54243328 | 1024 |

## Qualifications

- First repetition is first-pass; later repetitions are warm-cache. No cache eviction was claimed.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
