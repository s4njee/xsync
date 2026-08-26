# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787723503004467000`
- Source revision: `c142c6776aa1f82b4b07a3b0e69928716227539f-dirty`
- Build: `e183ba384a4f7636b0a19b8c3b24216c` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `local`
- Route: `same-volume`
- Shaping: `none`
- Streams: `1`
- Compression: `none (local route)`

## Corpus

- Schema: `xsync.manifest.v1`
- Manifest: `f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`
- Description: real corpus=congress-10k pinned_digest=f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6

## Tools

- **xsync** `xs 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 7.335741s | 0.679129s | 6.314534s | 56164352 | 22568 | 96542108 | 13160511.476 B/s | 0 | - | 5 | pass |
| xsync | 8.217143s | 0.387758s | 7.012791s | 36945920 | 22568 | 96542108 | 11748864.797 B/s | 0 | 0.867x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 9.383258s | 6.466297s | 47677440 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 6.656612s | 5.985849s | 51527680 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 7.335741s | 6.116131s | 56164352 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 6.136945s | 6.427313s | 55312384 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 7.464298s | 6.314534s | 50839552 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 7.829386s | 7.012791s | 35389440 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 8.217143s | 6.979267s | 36356096 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 7.862146s | 7.008529s | 36225024 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 9.519828s | 7.143615s | 35602432 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 8.605798s | 7.191834s | 36945920 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
