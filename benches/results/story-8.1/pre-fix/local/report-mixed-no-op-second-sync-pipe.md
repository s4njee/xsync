# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565603679799000`
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
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=no-op-second-sync seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.074203s | 0.002092s | 0.021924s | 5095424 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.071991s | 0.001811s | 0.022084s | 5095424 | 513 | 1769997 | 0 | 1.011x | 5 | pass |
| xsync | 0.042390s | 0.000280s | 0.028797s | 4653056 | 513 | 1769997 | 0 | 1.695x | 5 | pass |
| xsync-raw | 0.041277s | 0.000829s | 0.026651s | 4767744 | 513 | 1769997 | 0 | 1.782x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.072779s | 0.021509s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.076295s | 0.021215s | 5062656 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.074203s | 0.021924s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.071846s | 0.022169s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.078783s | 0.026237s | 5095424 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.071991s | 0.022555s | 5062656 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.070180s | 0.022084s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.073799s | 0.021842s | 5046272 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.066764s | 0.021903s | 5079040 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.081374s | 0.031517s | 5095424 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.042187s | 0.026698s | 4423680 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 1 | warm | 0.045012s | 0.030215s | 4620288 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 0 | warm | 0.042111s | 0.030858s | 4653056 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 3 | warm | 0.042390s | 0.026299s | 4603904 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 2 | warm | 0.060313s | 0.028797s | 4554752 | 513 | 1769997 | 0 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.040839s | 0.031103s | 4767744 | 513 | 1769997 | 0 | pass |
| xsync-raw | 1 | 2 | warm | 0.041277s | 0.026579s | 4587520 | 513 | 1769997 | 0 | pass |
| xsync-raw | 2 | 1 | warm | 0.044471s | 0.026651s | 4636672 | 513 | 1769997 | 0 | pass |
| xsync-raw | 3 | 0 | warm | 0.042105s | 0.026537s | 4423680 | 513 | 1769997 | 0 | pass |
| xsync-raw | 4 | 3 | warm | 0.034521s | 0.027890s | 4669440 | 513 | 1769997 | 0 | pass |
