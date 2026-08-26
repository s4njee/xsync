# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787580785260141000`
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
| rsync-a | 3.123413s | 0.047297s | 5.166530s | 54132736 | 22568 | 96542108 | 0 | - | 5 | pass |
| xsync | 6.724379s | 0.066091s | 28.751975s | 41664512 | 22568 | 96542108 | 0 | 0.471x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 3.299695s | 5.190702s | 48545792 | 22568 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 3.114641s | 5.017626s | 53673984 | 22568 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 3.123413s | 5.166530s | 49332224 | 22568 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 6.325475s | 5.806899s | 54132736 | 22568 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 3.076116s | 5.001451s | 51773440 | 22568 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 6.790470s | 28.751975s | 36388864 | 22568 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 6.697029s | 28.662629s | 35536896 | 22568 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 6.634835s | 28.868910s | 40026112 | 22568 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 6.974287s | 28.891388s | 41238528 | 22568 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 6.724379s | 28.484667s | 41664512 | 22568 | 96542108 | 0 | pass |
