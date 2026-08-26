# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568179726204000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

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
| rsync-a | 0.071631s | 0.001008s | 0.020446s | 5095424 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.062689s | 0.006519s | 0.020237s | 5079040 | 513 | 1769997 | 0 | 1.126x | 5 | pass |
| xsync | 0.038972s | 0.001495s | 0.024980s | 4653056 | 513 | 1769997 | 0 | 1.730x | 5 | pass |
| xsync-raw | 0.035665s | 0.001655s | 0.025658s | 4505600 | 513 | 1769997 | 0 | 1.924x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.071631s | 0.020902s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.071991s | 0.020446s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.072639s | 0.019568s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.050673s | 0.019765s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.066545s | 0.020803s | 5095424 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.069208s | 0.020526s | 5013504 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.062689s | 0.019568s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.048235s | 0.021463s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.045022s | 0.019489s | 5029888 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.068963s | 0.020237s | 5079040 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.049357s | 0.027302s | 4554752 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 1 | warm | 0.037477s | 0.024523s | 4423680 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 0 | warm | 0.040852s | 0.027657s | 4571136 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 3 | warm | 0.038972s | 0.024813s | 4489216 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 2 | warm | 0.038461s | 0.024980s | 4653056 | 513 | 1769997 | 0 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.032970s | 0.023245s | 4456448 | 513 | 1769997 | 0 | pass |
| xsync-raw | 1 | 2 | warm | 0.035665s | 0.026959s | 4390912 | 513 | 1769997 | 0 | pass |
| xsync-raw | 2 | 1 | warm | 0.037749s | 0.025658s | 4489216 | 513 | 1769997 | 0 | pass |
| xsync-raw | 3 | 0 | warm | 0.034009s | 0.026655s | 4505600 | 513 | 1769997 | 0 | pass |
| xsync-raw | 4 | 3 | warm | 0.036249s | 0.024575s | 4308992 | 513 | 1769997 | 0 | pass |
