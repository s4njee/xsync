# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568225785462000`
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
- Manifest: `96569fdd632cdf850e63d91483d67cbc7f8755efcbc77a3d8584c9584e1970ba`
- Description: class=one-large-file tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.084626s | 0.002973s | 0.030990s | 5341184 | 2 | 8388608 | 0 | - | 5 | pass |
| rsync-az | 0.085225s | 0.001707s | 0.035097s | 8896512 | 2 | 8388608 | 0 | 0.973x | 5 | pass |
| xsync | 0.053611s | 0.001638s | 0.038047s | 42385408 | 2 | 8388608 | 8388660 | 1.386x | 5 | pass |
| xsync-raw | 0.052967s | 0.001253s | 0.036704s | 39321600 | 2 | 8388608 | 8388660 | 1.589x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.087599s | 0.030990s | 5308416 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.059398s | 0.030209s | 5324800 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.086106s | 0.035021s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.084626s | 0.030972s | 5308416 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.060422s | 0.032823s | 5341184 | 2 | 8388608 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.085772s | 0.036487s | 8847360 | 2 | 8388608 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.064317s | 0.035097s | 8830976 | 2 | 8388608 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.066893s | 0.042578s | 8896512 | 2 | 8388608 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.086933s | 0.034751s | 8830976 | 2 | 8388608 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.085225s | 0.035029s | 8830976 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.063192s | 0.040795s | 42385408 | 2 | 8388608 | 8388660 | pass |
| xsync | 1 | 1 | warm | 0.053611s | 0.038047s | 40189952 | 2 | 8388608 | 8388660 | pass |
| xsync | 2 | 0 | warm | 0.049567s | 0.037939s | 40222720 | 2 | 8388608 | 8388660 | pass |
| xsync | 3 | 3 | warm | 0.052170s | 0.037887s | 40206336 | 2 | 8388608 | 8388660 | pass |
| xsync | 4 | 2 | warm | 0.055249s | 0.039221s | 40206336 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.055128s | 0.037110s | 39124992 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 1 | 2 | warm | 0.054220s | 0.037367s | 39223296 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 2 | 1 | warm | 0.049767s | 0.036457s | 39157760 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 3 | 0 | warm | 0.052967s | 0.036704s | 39321600 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 4 | 3 | warm | 0.052500s | 0.035957s | 39141376 | 2 | 8388608 | 8388660 | pass |
