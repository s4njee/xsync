# xsync scanner/planner evidence

- Schema: `xsync.engine-bench.report.v1`
- Shape: `flat-small-100k`
- Corpus manifest: `19e0b6ec9c89a540dbd793ef72f5bfdcffd941e4cad5da55d873b63ec24173f3`
- Host/OS: `MacBookPro` / `macos` / `Darwin 25.6.0`
- Hardware: `Apple M1 Max`
- Filesystem: `apfs`
- Memory budget: 536870912 bytes — **within budget**

## Summary

| Metric | Value |
|---|---:|
| Scan rate median | 637230 entries/s |
| Scan rate MAD | 1223 entries/s |
| Syscall-sensitive scan median | 0.313858 s |
| Destination index median | 0.007905 s |
| Planner median | 0.017016 s |
| Peak RSS | 281264128 bytes |
| Queue high-water | 1024 / 1024 |

## Repetitions

| Rep | Items | Scan entries/s | Syscall phase (s) | Index (s) | Plan (s) | Peak RSS | Queue HWM |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 100000 | 637230 | 0.313858 | 0.007903 | 0.019330 | 279969792 | 1024 |
| 1 | 100000 | 638454 | 0.313257 | 0.007823 | 0.011541 | 280887296 | 1024 |
| 2 | 100000 | 629432 | 0.317747 | 0.008487 | 0.016712 | 281264128 | 1024 |
| 3 | 100000 | 641337 | 0.311849 | 0.007905 | 0.017016 | 280559616 | 1024 |
| 4 | 100000 | 636993 | 0.313975 | 0.008082 | 0.017054 | 280526848 | 1024 |

## Qualifications

- First repetition is first-pass; later repetitions are warm-cache. No cache eviction was claimed.
- Peak RSS is measured in a fresh process for every repetition (Linux VmHWM; macOS /usr/bin/time -l).
- The portable scanner currently rejects non-UTF-8 paths; Story 2.1b remains required before protocol freeze.
