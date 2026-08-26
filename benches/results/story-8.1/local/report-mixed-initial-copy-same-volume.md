# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568057304111000`
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
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.140609s | 0.002432s | 0.114868s | 5390336 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.240602s | 0.032004s | 1.123391s | 5062656 | 513 | 1769997 | 0 | 0.534x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.118647s | 0.110643s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.143041s | 0.120562s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.140609s | 0.114450s | 5357568 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.119471s | 0.114868s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.140771s | 0.116438s | 5373952 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.272607s | 1.074001s | 5029888 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.267973s | 1.123391s | 5062656 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.202327s | 1.115086s | 4833280 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.240602s | 1.132746s | 4931584 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.204463s | 1.145811s | 4915200 | 513 | 1769997 | 0 | pass |
