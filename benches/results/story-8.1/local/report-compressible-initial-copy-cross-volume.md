# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568166955142000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk11s1)`
- Transport: `local`
- Route: `cross-volume`
- Streams: `1`
- Compression: `none (local route)`

## Corpus

- Schema: `xsync.corpus.v1`
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.044051s | 0.001168s | 0.021667s | 5210112 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.030068s | 0.001358s | 0.095660s | 4259840 | 33 | 2097152 | 0 | 1.525x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.046207s | 0.020701s | 5079040 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.065986s | 0.023079s | 5210112 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.044051s | 0.022875s | 5210112 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.042883s | 0.020775s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.043774s | 0.021667s | 5095424 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.028348s | 0.093976s | 4030464 | 33 | 2097152 | 0 | pass |
| xsync | 1 | 0 | warm | 0.031015s | 0.093897s | 3997696 | 33 | 2097152 | 0 | pass |
| xsync | 2 | 1 | warm | 0.034916s | 0.102086s | 4259840 | 33 | 2097152 | 0 | pass |
| xsync | 3 | 0 | warm | 0.030068s | 0.097966s | 4030464 | 33 | 2097152 | 0 | pass |
| xsync | 4 | 1 | warm | 0.028710s | 0.095660s | 4063232 | 33 | 2097152 | 0 | pass |
