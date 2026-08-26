# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565559179815000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.237635s | 0.003215s | 0.229192s | 5242880 | 1001 | 61380 | 0 | - | 5 | pass |
| xsync | 0.470198s | 0.020330s | 2.444523s | 7962624 | 1001 | 61380 | 0 | 0.527x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.236384s | 0.220521s | 5226496 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.263660s | 0.232910s | 5242880 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.237635s | 0.228836s | 5144576 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.268136s | 0.240302s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.234420s | 0.229192s | 5193728 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.448510s | 2.481234s | 7766016 | 1001 | 61380 | 0 | pass |
| xsync | 1 | 0 | warm | 0.466693s | 2.474215s | 7913472 | 1001 | 61380 | 0 | pass |
| xsync | 2 | 1 | warm | 0.497631s | 2.374785s | 7962624 | 1001 | 61380 | 0 | pass |
| xsync | 3 | 0 | warm | 0.470198s | 2.413009s | 7946240 | 1001 | 61380 | 0 | pass |
| xsync | 4 | 1 | warm | 0.490528s | 2.444523s | 7716864 | 1001 | 61380 | 0 | pass |
