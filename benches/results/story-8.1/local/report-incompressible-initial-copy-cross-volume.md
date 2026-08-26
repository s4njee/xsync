# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568167863218000`
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
- Manifest: `904339c1374d49e04de1263354978dd256bfbe468332911b6353ced6b6b71074`
- Description: class=incompressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.043451s | 0.000554s | 0.020475s | 5177344 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.033886s | 0.001345s | 0.097339s | 4063232 | 33 | 2097152 | 0 | 1.387x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.044264s | 0.020123s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.043341s | 0.020957s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.043451s | 0.020999s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.042897s | 0.020389s | 5111808 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.064254s | 0.020475s | 5062656 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.033886s | 0.107679s | 3981312 | 33 | 2097152 | 0 | pass |
| xsync | 1 | 0 | warm | 0.030165s | 0.097339s | 4063232 | 33 | 2097152 | 0 | pass |
| xsync | 2 | 1 | warm | 0.031334s | 0.097118s | 4046848 | 33 | 2097152 | 0 | pass |
| xsync | 3 | 0 | warm | 0.033896s | 0.093284s | 4046848 | 33 | 2097152 | 0 | pass |
| xsync | 4 | 1 | warm | 0.035231s | 0.102311s | 4046848 | 33 | 2097152 | 0 | pass |
