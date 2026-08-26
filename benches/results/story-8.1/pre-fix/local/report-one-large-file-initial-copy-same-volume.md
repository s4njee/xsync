# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565562326080000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.050085s | 0.000087s | 0.025215s | 5341184 | 2 | 8388608 | 0 | - | 5 | pass |
| xsync | 0.019963s | 0.000985s | 0.009250s | 3850240 | 2 | 8388608 | 0 | 2.599x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.049998s | 0.025215s | 5324800 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.073240s | 0.027139s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.050119s | 0.024790s | 5341184 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.050085s | 0.026432s | 5275648 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.047603s | 0.024075s | 5308416 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.018979s | 0.009250s | 3833856 | 2 | 8388608 | 0 | pass |
| xsync | 1 | 0 | warm | 0.021700s | 0.009193s | 3850240 | 2 | 8388608 | 0 | pass |
| xsync | 2 | 1 | warm | 0.021208s | 0.009116s | 3735552 | 2 | 8388608 | 0 | pass |
| xsync | 3 | 0 | warm | 0.019267s | 0.009459s | 3686400 | 2 | 8388608 | 0 | pass |
| xsync | 4 | 1 | warm | 0.019963s | 0.009372s | 3702784 | 2 | 8388608 | 0 | pass |
