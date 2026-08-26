# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568100163050000`
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
- Manifest: `b821f36f723f499539ba22d3014b4a62f6a6457149cb131dbe953dbced3baaa6`
- Description: class=mixed tier=smoke workload=type-replacement seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.036892s | 0.000547s | 0.015161s | 5095424 | 513 | 1749572 | 0 | - | 5 | pass |
| xsync | 0.030657s | 0.002626s | 0.019734s | 5308416 | 513 | 1749572 | 0 | 1.297x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.036345s | 0.014596s | 5013504 | 513 | 1749572 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.036892s | 0.015161s | 5013504 | 513 | 1749572 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.036530s | 0.014191s | 5062656 | 513 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.038659s | 0.015689s | 5095424 | 513 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.063524s | 0.015923s | 5062656 | 513 | 1749572 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.028032s | 0.021393s | 5160960 | 513 | 1749572 | 0 | pass |
| xsync | 1 | 0 | warm | 0.037823s | 0.018695s | 5013504 | 513 | 1749572 | 0 | pass |
| xsync | 2 | 1 | warm | 0.035205s | 0.019387s | 5210112 | 513 | 1749572 | 0 | pass |
| xsync | 3 | 0 | warm | 0.028530s | 0.019734s | 5111808 | 513 | 1749572 | 0 | pass |
| xsync | 4 | 1 | warm | 0.030657s | 0.022659s | 5308416 | 513 | 1749572 | 0 | pass |
