# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565617139485000`
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
- Manifest: `e8ad8d9b19b4589c777af4089f02b3c8a0e0f04288b674813b9f58b7c98ffc12`
- Description: class=mixed tier=smoke workload=metadata-only-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.076114s | 0.002972s | 0.023463s | 5390336 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.069131s | 0.004210s | 0.023885s | 5472256 | 513 | 1769997 | 0 | 1.068x | 5 | pass |
| xsync | 0.041927s | 0.000934s | 0.029863s | 4915200 | 513 | 1769997 | 4128 | 1.805x | 5 | pass |
| xsync-raw | 0.042808s | 0.001459s | 0.028739s | 4751360 | 513 | 1769997 | 20633 | 1.784x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.079087s | 0.022625s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.076114s | 0.024014s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.076372s | 0.023463s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.069289s | 0.023191s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.070427s | 0.024776s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.047703s | 0.024403s | 5439488 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.073341s | 0.023572s | 5472256 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.071501s | 0.023885s | 5472256 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.069131s | 0.025114s | 5455872 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.052444s | 0.023841s | 5406720 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.043810s | 0.027539s | 4751360 | 513 | 1769997 | 4128 | pass |
| xsync | 1 | 1 | warm | 0.041753s | 0.030895s | 4734976 | 513 | 1769997 | 4128 | pass |
| xsync | 2 | 0 | warm | 0.040992s | 0.030541s | 4751360 | 513 | 1769997 | 4128 | pass |
| xsync | 3 | 3 | warm | 0.041927s | 0.029863s | 4915200 | 513 | 1769997 | 4128 | pass |
| xsync | 4 | 2 | warm | 0.046906s | 0.029403s | 4751360 | 513 | 1769997 | 4128 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.043243s | 0.028739s | 4636672 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 1 | 2 | warm | 0.041350s | 0.027508s | 4734976 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 2 | 1 | warm | 0.042808s | 0.030167s | 4702208 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 3 | 0 | warm | 0.045192s | 0.028743s | 4538368 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 4 | 3 | warm | 0.040770s | 0.028182s | 4751360 | 513 | 1769997 | 20633 | pass |
