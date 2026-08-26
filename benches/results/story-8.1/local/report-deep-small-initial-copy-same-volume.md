# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568109227086000`
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
- Manifest: `c4bbb1d39a822958eef248d6a45f442e3bb35ff00353094e853e2c88a33c6c8a`
- Description: class=deep-small tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.263925s | 0.002148s | 0.231579s | 5210112 | 1001 | 61380 | 0 | - | 5 | pass |
| xsync | 0.453089s | 0.009327s | 2.460083s | 8110080 | 1001 | 61380 | 0 | 0.578x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.261777s | 0.227802s | 5177344 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.231295s | 0.225307s | 5193728 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.265382s | 0.241146s | 5193728 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.267459s | 0.244171s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.263925s | 0.231579s | 5210112 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.453089s | 2.500753s | 7634944 | 1001 | 61380 | 0 | pass |
| xsync | 1 | 0 | warm | 0.404991s | 2.460083s | 7897088 | 1001 | 61380 | 0 | pass |
| xsync | 2 | 1 | warm | 0.462416s | 2.402752s | 8110080 | 1001 | 61380 | 0 | pass |
| xsync | 3 | 0 | warm | 0.460575s | 2.416013s | 7946240 | 1001 | 61380 | 0 | pass |
| xsync | 4 | 1 | warm | 0.429958s | 2.491158s | 7684096 | 1001 | 61380 | 0 | pass |
