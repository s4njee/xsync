# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568192718132000`
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
- Manifest: `e8ad8d9b19b4589c777af4089f02b3c8a0e0f04288b674813b9f58b7c98ffc12`
- Description: class=mixed tier=smoke workload=metadata-only-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.071685s | 0.003939s | 0.021635s | 5390336 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.072527s | 0.001807s | 0.022130s | 5472256 | 513 | 1769997 | 0 | 0.986x | 5 | pass |
| xsync | 0.040084s | 0.001189s | 0.026624s | 4767744 | 513 | 1769997 | 4128 | 1.740x | 5 | pass |
| xsync-raw | 0.039786s | 0.000777s | 0.027150s | 4620288 | 513 | 1769997 | 20633 | 1.782x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.075624s | 0.021635s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.052577s | 0.022561s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.066118s | 0.021544s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.073295s | 0.021410s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.071685s | 0.022912s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.065710s | 0.021463s | 5406720 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.050450s | 0.022130s | 5439488 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.072527s | 0.022209s | 5423104 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.074334s | 0.021863s | 5455872 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.074170s | 0.023354s | 5472256 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.040084s | 0.026624s | 4751360 | 513 | 1769997 | 4128 | pass |
| xsync | 1 | 1 | warm | 0.037147s | 0.024732s | 4603904 | 513 | 1769997 | 4128 | pass |
| xsync | 2 | 0 | warm | 0.038895s | 0.025518s | 4653056 | 513 | 1769997 | 4128 | pass |
| xsync | 3 | 3 | warm | 0.042124s | 0.030759s | 4620288 | 513 | 1769997 | 4128 | pass |
| xsync | 4 | 2 | warm | 0.041074s | 0.027746s | 4767744 | 513 | 1769997 | 4128 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.037421s | 0.027568s | 4521984 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 1 | 2 | warm | 0.039786s | 0.027150s | 4472832 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 2 | 1 | warm | 0.040563s | 0.026557s | 4620288 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 3 | 0 | warm | 0.038383s | 0.026261s | 4505600 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 4 | 3 | warm | 0.040228s | 0.028571s | 4521984 | 513 | 1769997 | 20633 | pass |
