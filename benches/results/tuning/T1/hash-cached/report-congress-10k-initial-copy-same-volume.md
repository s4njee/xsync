# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787582024891428000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `7d5798f6fbc04b0466d21decd4369265` (`release`)

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
| rsync-a | 3.185598s | 0.048073s | 5.191894s | 38813696 | 22568 | 96542108 | 30305804.607 B/s | 0 | - | 5 | pass |
| xsync | 3.070423s | 0.141528s | 10.250955s | 29589504 | 22568 | 96542108 | 31442605.406 B/s | 0 | 1.020x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 3.230090s | 5.153927s | 38682624 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 3.137525s | 5.176927s | 38731776 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 3.130947s | 5.191894s | 38715392 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 3.185598s | 5.202683s | 38748160 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 3.277326s | 5.341505s | 38813696 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 3.243378s | 10.250955s | 29442048 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 3.103772s | 10.188198s | 28540928 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 3.070423s | 9.761167s | 29507584 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 2.915126s | 10.704591s | 29491200 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 2.928895s | 10.537382s | 29589504 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
