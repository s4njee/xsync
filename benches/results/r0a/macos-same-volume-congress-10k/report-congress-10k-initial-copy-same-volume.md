# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1788211132658250000`
- Source revision: `17474b6ca0545d288f8d18e19061bb08412d3536-dirty`
- Build: `3438717077cf0b1e93ca6e7244c37f7e` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.2-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `local`
- Route: `macos-apfs-same-volume`
- Shaping: `none`
- Streams: `1`
- Compression: `none (local route)`

## Corpus

- Schema: `xsync.manifest.v1`
- Manifest: `f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`
- Description: real corpus=congress-10k pinned_digest=f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6

## Tools

- **xsync** `xs 0.1.0 (a6410601318b-dirty 2026-08-31) aarch64-apple-darwin`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 3.342511s | 0.016765s | 4.932584s | 52805632 | 22568 | 96542108 | 28883104.217 B/s | 0 | - | 3 | pass |
| xsync | 1.360277s | 0.032773s | 2.275916s | 52838400 | 22568 | 96542108 | 70972401.755 B/s | 0 | 2.470x | 3 | pass |

## Repetitions

| method | rep | order | cache | wall | durable | CPU | endpoint CPU | RSS | endpoint RSS | cache resident/total | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 3.342511s | 4.337680s | 4.847101s | 0.000000s | 46612480 | 0 | 96542108/96542108 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 3.359277s | 4.186102s | 4.939649s | 0.000000s | 50593792 | 0 | 96542108/96542108 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 3.203765s | 4.290501s | 4.932584s | 0.000000s | 52805632 | 0 | 96542108/96542108 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 1.306421s | 2.270751s | 2.327910s | 0.000000s | 48988160 | 0 | 6148/96542108 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 1.360277s | 2.325793s | 2.205990s | 0.000000s | 52838400 | 0 | 6148/96542108 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 1.393050s | 2.419232s | 2.275916s | 0.000000s | 52314112 | 0 | 6148/96542108 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
