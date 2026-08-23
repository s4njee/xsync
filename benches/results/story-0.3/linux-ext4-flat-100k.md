# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `flat-small-100k`
- Corpus manifest: `19e0b6ec9c89a540dbd793ef72f5bfdcffd941e4cad5da55d873b63ec24173f3`
- Host/OS: `mars` / `linux` / `Linux 7.1.6-arch1-1`
- Hardware: `AMD Ryzen 9 7900X 12-Core Processor`
- Filesystem: `ext4`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 1132801 entries/s |
| Scan rate MAD | 47802 entries/s |
| Syscall-sensitive scan median | 0.176553 s |
| Destination index median | 0.009333 s |
| Planner median | 0.011624 s |
| Peak RSS | 54542336 bytes |
| Queue high-water | 1022 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100000 | 1132801 | 0.176553 | 0.009333 | 0.010950 | 50368512 | 705 |
| 1 | 100000 | 1047394 | 0.190950 | 0.009036 | 0.011624 | 49590272 | 714 |
| 2 | 100000 | 1089642 | 0.183546 | 0.008253 | 0.010303 | 48795648 | 996 |
| 3 | 100000 | 1180603 | 0.169405 | 0.014529 | 0.017732 | 54542336 | 709 |
| 4 | 100000 | 1194357 | 0.167454 | 0.013790 | 0.017723 | 50991104 | 1022 |

## Qualifications

- First repetition is first-pass; later repetitions are warm-cache. No cache eviction was claimed.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
