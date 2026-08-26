# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568110278870000`
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
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.046441s | 0.002768s | 0.021421s | 5226496 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.033852s | 0.000896s | 0.096178s | 4128768 | 33 | 2097152 | 0 | 1.493x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.046441s | 0.020853s | 5128192 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.043673s | 0.021601s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.066684s | 0.021421s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.045476s | 0.021241s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.070329s | 0.021430s | 5226496 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.031099s | 0.087693s | 4079616 | 33 | 2097152 | 0 | pass |
| xsync | 1 | 0 | warm | 0.033018s | 0.096178s | 4128768 | 33 | 2097152 | 0 | pass |
| xsync | 2 | 1 | warm | 0.034748s | 0.098752s | 4112384 | 33 | 2097152 | 0 | pass |
| xsync | 3 | 0 | warm | 0.036924s | 0.079738s | 4030464 | 33 | 2097152 | 0 | pass |
| xsync | 4 | 1 | warm | 0.033852s | 0.098529s | 4112384 | 33 | 2097152 | 0 | pass |
