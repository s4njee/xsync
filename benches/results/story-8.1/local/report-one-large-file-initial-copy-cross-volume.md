# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568168791984000`
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
- Manifest: `96569fdd632cdf850e63d91483d67cbc7f8755efcbc77a3d8584c9584e1970ba`
- Description: class=one-large-file tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.055036s | 0.000746s | 0.023439s | 5292032 | 2 | 8388608 | 0 | - | 5 | pass |
| xsync | 0.031588s | 0.000821s | 0.011307s | 3850240 | 2 | 8388608 | 0 | 1.748x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.055782s | 0.023439s | 5242880 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.055036s | 0.022904s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.053933s | 0.024165s | 5242880 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.054426s | 0.025151s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.057222s | 0.022753s | 5242880 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.032888s | 0.010945s | 3801088 | 2 | 8388608 | 0 | pass |
| xsync | 1 | 0 | warm | 0.029889s | 0.011307s | 3702784 | 2 | 8388608 | 0 | pass |
| xsync | 2 | 1 | warm | 0.030861s | 0.012171s | 3850240 | 2 | 8388608 | 0 | pass |
| xsync | 3 | 0 | warm | 0.032409s | 0.011582s | 3686400 | 2 | 8388608 | 0 | pass |
| xsync | 4 | 1 | warm | 0.031588s | 0.011260s | 3686400 | 2 | 8388608 | 0 | pass |
