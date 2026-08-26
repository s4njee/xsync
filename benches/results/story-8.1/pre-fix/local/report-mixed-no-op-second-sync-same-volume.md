# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565540741636000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `local`
- Route: `same-volume`
- Streams: `1`
- Compression: `none (local route)`

## Corpus

- Schema: `xsync.corpus.v1`
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=no-op-second-sync seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.058303s | 0.004292s | 0.014042s | 5095424 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.027755s | 0.000980s | 0.020741s | 5242880 | 513 | 1769997 | 0 | 1.975x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.061371s | 0.013568s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.037177s | 0.014685s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.062595s | 0.013989s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.039236s | 0.014315s | 5062656 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.058303s | 0.014042s | 5095424 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.031072s | 0.020741s | 5128192 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.030528s | 0.020403s | 5046272 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.026775s | 0.022284s | 5062656 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.027755s | 0.020153s | 5210112 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.027725s | 0.021076s | 5242880 | 513 | 1769997 | 0 | pass |
