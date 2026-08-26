# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568092981155000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

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
- Manifest: `e8ad8d9b19b4589c777af4089f02b3c8a0e0f04288b674813b9f58b7c98ffc12`
- Description: class=mixed tier=smoke workload=metadata-only-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.049597s | 0.013268s | 0.015907s | 5111808 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.037618s | 0.000688s | 0.032747s | 5292032 | 513 | 1769997 | 0 | 1.064x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.035315s | 0.014317s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.062865s | 0.015414s | 5095424 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.040558s | 0.015907s | 5111808 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.072825s | 0.020234s | 5095424 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.049597s | 0.019020s | 5111808 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.037618s | 0.031737s | 5046272 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.036930s | 0.030863s | 5292032 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.038128s | 0.035380s | 5079040 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.035705s | 0.033980s | 4980736 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.054717s | 0.032747s | 5242880 | 513 | 1769997 | 0 | pass |
