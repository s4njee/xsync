# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568156148084000`
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
- Manifest: `b821f36f723f499539ba22d3014b4a62f6a6457149cb131dbe953dbced3baaa6`
- Description: class=mixed tier=smoke workload=type-replacement seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.039833s | 0.002705s | 0.015175s | 5079040 | 513 | 1749572 | 0 | - | 5 | pass |
| xsync | 0.026225s | 0.000463s | 0.021095s | 5275648 | 513 | 1749572 | 0 | 1.542x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.039833s | 0.015077s | 5079040 | 513 | 1749572 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.061090s | 0.015403s | 5079040 | 513 | 1749572 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.039853s | 0.015038s | 5079040 | 513 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.036295s | 0.015233s | 5062656 | 513 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.037128s | 0.015175s | 5029888 | 513 | 1749572 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.025838s | 0.021095s | 5275648 | 513 | 1749572 | 0 | pass |
| xsync | 1 | 0 | warm | 0.026225s | 0.023491s | 5029888 | 513 | 1749572 | 0 | pass |
| xsync | 2 | 1 | warm | 0.026689s | 0.020716s | 5160960 | 513 | 1749572 | 0 | pass |
| xsync | 3 | 0 | warm | 0.028772s | 0.019789s | 5095424 | 513 | 1749572 | 0 | pass |
| xsync | 4 | 1 | warm | 0.022597s | 0.022090s | 4980736 | 513 | 1749572 | 0 | pass |
