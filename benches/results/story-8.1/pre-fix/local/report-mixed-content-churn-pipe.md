# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565610550368000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `pipe (child xsync --server over stdio)`
- Route: `pipe`
- Streams: `1`
- Compression: `adaptive zstd`

## Corpus

- Schema: `xsync.corpus.v1`
- Manifest: `17bfa0714453dc63deddbcd7b602cf0ed002367c36508ebbe5e730038d873880`
- Description: class=mixed tier=smoke workload=content-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.101205s | 0.005893s | 0.045473s | 5668864 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.084609s | 0.008238s | 0.045830s | 6766592 | 513 | 1769997 | 0 | 1.027x | 5 | pass |
| xsync | 0.076924s | 0.001886s | 0.057872s | 5046272 | 513 | 1769997 | 4131 | 1.108x | 5 | pass |
| xsync-raw | 0.077682s | 0.001660s | 0.057658s | 4800512 | 513 | 1769997 | 20633 | 1.303x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.106311s | 0.044920s | 5603328 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.083176s | 0.045895s | 5652480 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.107097s | 0.048127s | 5652480 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.079150s | 0.045189s | 5668864 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.101205s | 0.045473s | 5636096 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.103524s | 0.046485s | 6701056 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.081970s | 0.044730s | 6766592 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.084609s | 0.048720s | 6668288 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.103531s | 0.045830s | 6668288 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.076371s | 0.044936s | 6684672 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.097540s | 0.062937s | 4980736 | 513 | 1769997 | 4131 | pass |
| xsync | 1 | 1 | warm | 0.075037s | 0.058603s | 5046272 | 513 | 1769997 | 4131 | pass |
| xsync | 2 | 0 | warm | 0.075320s | 0.055052s | 4915200 | 513 | 1769997 | 4131 | pass |
| xsync | 3 | 3 | warm | 0.076924s | 0.054982s | 4833280 | 513 | 1769997 | 4131 | pass |
| xsync | 4 | 2 | warm | 0.078896s | 0.057872s | 4997120 | 513 | 1769997 | 4131 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.077783s | 0.054417s | 4620288 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 1 | 2 | warm | 0.093125s | 0.059055s | 4800512 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 2 | 1 | warm | 0.076022s | 0.057658s | 4718592 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 3 | 0 | warm | 0.075542s | 0.058286s | 4800512 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 4 | 3 | warm | 0.077682s | 0.055692s | 4767744 | 513 | 1769997 | 20633 | pass |
