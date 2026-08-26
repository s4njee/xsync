# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565591893276000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.054910s | 0.000353s | 0.025397s | 5324800 | 2 | 8388608 | 0 | - | 5 | pass |
| xsync | 0.032389s | 0.000468s | 0.011436s | 3866624 | 2 | 8388608 | 0 | 1.688x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.054910s | 0.025397s | 5324800 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.055263s | 0.026703s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.053203s | 0.025321s | 5324800 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.056621s | 0.026413s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.054676s | 0.024456s | 5259264 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.035987s | 0.011466s | 3850240 | 2 | 8388608 | 0 | pass |
| xsync | 1 | 0 | warm | 0.030779s | 0.011459s | 3702784 | 2 | 8388608 | 0 | pass |
| xsync | 2 | 1 | warm | 0.032263s | 0.011215s | 3850240 | 2 | 8388608 | 0 | pass |
| xsync | 3 | 0 | warm | 0.032858s | 0.010913s | 3866624 | 2 | 8388608 | 0 | pass |
| xsync | 4 | 1 | warm | 0.032389s | 0.011436s | 3866624 | 2 | 8388608 | 0 | pass |
