# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568118853080000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk11s1)`
- Transport: `local`
- Route: `cross-volume`
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
| rsync-a | 0.037033s | 0.000602s | 0.014190s | 5079040 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.022474s | 0.001045s | 0.018554s | 5357568 | 513 | 1769997 | 0 | 1.643x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.037444s | 0.013535s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.036424s | 0.015179s | 5062656 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.037033s | 0.014142s | 5062656 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.036431s | 0.014929s | 5046272 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.037678s | 0.014190s | 5013504 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.020579s | 0.018728s | 5193728 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.022474s | 0.017561s | 4997120 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.023995s | 0.016951s | 5046272 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.021429s | 0.018554s | 5193728 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.022930s | 0.020608s | 5357568 | 513 | 1769997 | 0 | pass |
