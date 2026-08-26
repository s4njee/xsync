# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787581907288957000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `24fa14114e9916fe6d7feace338117eb` (`release`)

## Environment

- Hardware: `sysctl: sysctl fmt -1 1024 1: Operation not permitted, sysctl: sysctl fmt -1 1024 1: Operation not permitted logical cores, 0 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `/dev/disk3s5`
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

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 3.267072s | 0.094123s | 5.231813s | 38862848 | 22568 | 96542108 | 29550039.542 B/s | 0 | - | 5 | pass |
| xsync | 2.992617s | 0.009252s | 10.013016s | 30146560 | 22568 | 96542108 | 32260090.265 B/s | 0 | 1.077x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 3.679192s | 5.648430s | 38862848 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 3.267072s | 5.231813s | 38780928 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 3.304637s | 5.297469s | 38682624 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 3.172949s | 5.212209s | 38731776 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 3.154394s | 5.212817s | 38764544 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 3.189092s | 10.013016s | 30146560 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 2.990318s | 10.196283s | 29917184 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 3.068547s | 9.963346s | 28983296 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 2.983365s | 10.603699s | 29147136 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 2.992617s | 10.007602s | 28000256 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
