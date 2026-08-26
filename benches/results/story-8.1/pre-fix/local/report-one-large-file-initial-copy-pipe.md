# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565639001786000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.082451s | 0.001010s | 0.032295s | 5324800 | 2 | 8388608 | 0 | - | 5 | pass |
| rsync-az | 0.084382s | 0.008135s | 0.036551s | 8880128 | 2 | 8388608 | 0 | 0.982x | 5 | pass |
| xsync | 0.053456s | 0.000541s | 0.038135s | 42500096 | 2 | 8388608 | 8388660 | 1.556x | 5 | pass |
| xsync-raw | 0.053103s | 0.000629s | 0.037854s | 39436288 | 2 | 8388608 | 8388660 | 1.553x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.084016s | 0.031480s | 5324800 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.059885s | 0.032295s | 5292032 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.083461s | 0.032646s | 5242880 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.082451s | 0.030838s | 5308416 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.082324s | 0.032548s | 5308416 | 2 | 8388608 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.092517s | 0.036679s | 8880128 | 2 | 8388608 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.060974s | 0.036551s | 8863744 | 2 | 8388608 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.084382s | 0.036147s | 8830976 | 2 | 8388608 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.086198s | 0.036742s | 8798208 | 2 | 8388608 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.063336s | 0.035529s | 8798208 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.053997s | 0.038656s | 42500096 | 2 | 8388608 | 8388660 | pass |
| xsync | 1 | 1 | warm | 0.051539s | 0.037264s | 40534016 | 2 | 8388608 | 8388660 | pass |
| xsync | 2 | 0 | warm | 0.053456s | 0.039520s | 40484864 | 2 | 8388608 | 8388660 | pass |
| xsync | 3 | 3 | warm | 0.053478s | 0.038135s | 40534016 | 2 | 8388608 | 8388660 | pass |
| xsync | 4 | 2 | warm | 0.051881s | 0.037774s | 42467328 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.053130s | 0.037463s | 39288832 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 1 | 2 | warm | 0.050296s | 0.037620s | 39354368 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 2 | 1 | warm | 0.053732s | 0.039523s | 39256064 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 3 | 0 | warm | 0.051688s | 0.039332s | 39436288 | 2 | 8388608 | 8388660 | pass |
| xsync-raw | 4 | 3 | warm | 0.053103s | 0.037854s | 39354368 | 2 | 8388608 | 8388660 | pass |
