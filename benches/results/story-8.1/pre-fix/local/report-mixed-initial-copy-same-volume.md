# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565537219684000`
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
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.122592s | 0.002078s | 0.118559s | 5439488 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.210209s | 0.003517s | 1.119831s | 5242880 | 513 | 1769997 | 0 | 0.582x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.129136s | 0.114155s | 5439488 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.124092s | 0.118914s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.120513s | 0.118559s | 5357568 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.117604s | 0.115929s | 5357568 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.122592s | 0.118777s | 5390336 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.218418s | 1.149206s | 5029888 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.210209s | 1.139610s | 5046272 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.207183s | 1.095071s | 4947968 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.206691s | 1.102029s | 5046272 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.220707s | 1.119831s | 5242880 | 513 | 1769997 | 0 | pass |
