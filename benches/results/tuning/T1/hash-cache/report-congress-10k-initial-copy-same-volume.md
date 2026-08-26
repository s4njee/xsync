# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787581784509268000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `d9d5654929be64612b520e76f893121c` (`release`)

## Environment

- Hardware: `sysctl: sysctl fmt -1 1024 1: Operation not permitted, sysctl: sysctl fmt -1 1024 1: Operation not permitted logical cores, 0 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `/dev/disk3s5`
- Transport: `local`
- Route: `same-volume`
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

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 3.620143s | 0.027319s | 5.717161s | 38846464 | 22568 | 96542108 | 0 | - | 5 | pass |
| xsync | 7.563641s | 0.182642s | 29.503535s | 26116096 | 22568 | 96542108 | 0 | 0.482x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 3.566893s | 5.624410s | 38715392 | 22568 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 3.700591s | 5.805489s | 38764544 | 22568 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 3.647463s | 5.732753s | 38699008 | 22568 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 3.602115s | 5.701611s | 38846464 | 22568 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 3.620143s | 5.717161s | 38682624 | 22568 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 7.587697s | 29.524781s | 25903104 | 22568 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 7.380998s | 29.404443s | 26116096 | 22568 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 7.563641s | 29.503535s | 25493504 | 22568 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 7.890980s | 29.537842s | 26099712 | 22568 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 6.868053s | 29.075648s | 25804800 | 22568 | 96542108 | 0 | pass |
