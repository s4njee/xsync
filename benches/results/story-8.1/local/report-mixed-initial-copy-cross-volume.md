# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568115145785000`
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
- Description: class=mixed tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.122014s | 0.001871s | 0.114455s | 5406720 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.215909s | 0.003127s | 1.218510s | 5111808 | 513 | 1769997 | 0 | 0.573x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.122014s | 0.113610s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.119702s | 0.114455s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.118698s | 0.112751s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.123885s | 0.116334s | 5406720 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.123544s | 0.115526s | 5390336 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.212782s | 1.185510s | 5095424 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.217722s | 1.251984s | 5111808 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.219736s | 1.231864s | 5079040 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.215909s | 1.203304s | 5111808 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.208777s | 1.218510s | 5046272 | 513 | 1769997 | 0 | pass |
