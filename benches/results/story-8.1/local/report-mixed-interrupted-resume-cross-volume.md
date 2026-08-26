# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568159936772000`
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
- Description: class=mixed tier=smoke workload=interrupted-resume seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.083342s | 0.002601s | 0.072319s | 5390336 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.132734s | 0.001439s | 0.634318s | 5406720 | 513 | 1769997 | 0 | 0.630x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.080741s | 0.066741s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.083114s | 0.071454s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.086304s | 0.075155s | 5357568 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.083342s | 0.072319s | 5324800 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.091382s | 0.074036s | 5357568 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.131564s | 0.635007s | 5275648 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.134218s | 0.632904s | 5406720 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.137028s | 0.639169s | 5357568 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.131296s | 0.634318s | 5357568 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.132734s | 0.621242s | 5226496 | 513 | 1769997 | 0 | pass |
