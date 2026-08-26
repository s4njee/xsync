# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787723292835121000`
- Source revision: `c142c6776aa1f82b4b07a3b0e69928716227539f-dirty`
- Build: `80848a74986b2b6c384be5eb120cf55a` (`release`)

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
| rsync-a | 5.294354s | 1.418510s | 6.212511s | 60211200 | 22568 | 96542108 | 18234915.701 B/s | 0 | - | 5 | pass |
| xsync | 39.608799s | 6.550218s | 29.387704s | 29687808 | 22568 | 96542108 | 2437390.439 B/s | 0 | 0.128x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 9.844956s | 6.533907s | 59736064 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 4.025928s | 5.782326s | 45727744 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 3.875844s | 5.663539s | 60211200 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 9.291242s | 6.493747s | 56688640 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 5.294354s | 6.212511s | 55918592 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 46.479284s | 30.890957s | 29687808 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 39.608799s | 29.387704s | 28786688 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 30.780888s | 27.284518s | 27721728 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 33.058581s | 28.388090s | 29392896 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 41.265690s | 31.007019s | 29278208 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
