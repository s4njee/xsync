# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787573696744886000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `804f39ed04815d2550b9146cc5ca7065` (`release`)

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
| rsync-a | 0.147111s | 0.005604s | 0.054340s | 5652480 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.152274s | 0.004923s | 0.053642s | 6717440 | 513 | 1769997 | 0 | 0.992x | 5 | pass |
| xsync | 0.122984s | 0.012279s | 0.065561s | 5046272 | 513 | 1769997 | 4131 | 1.229x | 5 | pass |
| xsync-raw | 0.108796s | 0.003538s | 0.063726s | 4816896 | 513 | 1769997 | 20633 | 1.356x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.158794s | 0.057202s | 5619712 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.141507s | 0.056406s | 5652480 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.147111s | 0.054340s | 5586944 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.151124s | 0.053213s | 5636096 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.135118s | 0.052926s | 5636096 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.129570s | 0.053642s | 6701056 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.157197s | 0.055809s | 6717440 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.120611s | 0.052834s | 6717440 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.152274s | 0.053474s | 6717440 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.156906s | 0.054749s | 6668288 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.135263s | 0.066519s | 4816896 | 513 | 1769997 | 4131 | pass |
| xsync | 1 | 1 | warm | 0.105311s | 0.064937s | 4964352 | 513 | 1769997 | 4131 | pass |
| xsync | 2 | 0 | warm | 0.111037s | 0.064179s | 5046272 | 513 | 1769997 | 4131 | pass |
| xsync | 3 | 3 | warm | 0.122984s | 0.065561s | 4833280 | 513 | 1769997 | 4131 | pass |
| xsync | 4 | 2 | warm | 0.149870s | 0.065851s | 4964352 | 513 | 1769997 | 4131 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.099926s | 0.063726s | 4734976 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 1 | 2 | warm | 0.112334s | 0.064842s | 4784128 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 2 | 1 | warm | 0.104670s | 0.063925s | 4767744 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 3 | 0 | warm | 0.111459s | 0.062212s | 4816896 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 4 | 3 | warm | 0.108796s | 0.061439s | 4751360 | 513 | 1769997 | 20633 | pass |
