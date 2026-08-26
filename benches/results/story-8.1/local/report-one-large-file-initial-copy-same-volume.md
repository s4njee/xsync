# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568112213337000`
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
- Manifest: `96569fdd632cdf850e63d91483d67cbc7f8755efcbc77a3d8584c9584e1970ba`
- Description: class=one-large-file tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.049707s | 0.003662s | 0.022830s | 5324800 | 2 | 8388608 | 0 | - | 5 | pass |
| xsync | 0.019804s | 0.000240s | 0.009015s | 3883008 | 2 | 8388608 | 0 | 2.541x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.071423s | 0.022829s | 5242880 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.046045s | 0.022580s | 5308416 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.071195s | 0.022830s | 5242880 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.049707s | 0.025133s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.047167s | 0.024298s | 5324800 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.019804s | 0.009367s | 3883008 | 2 | 8388608 | 0 | pass |
| xsync | 1 | 0 | warm | 0.020504s | 0.008576s | 3817472 | 2 | 8388608 | 0 | pass |
| xsync | 2 | 1 | warm | 0.022008s | 0.009482s | 3735552 | 2 | 8388608 | 0 | pass |
| xsync | 3 | 0 | warm | 0.019564s | 0.008931s | 3883008 | 2 | 8388608 | 0 | pass |
| xsync | 4 | 1 | warm | 0.019597s | 0.009015s | 3866624 | 2 | 8388608 | 0 | pass |
