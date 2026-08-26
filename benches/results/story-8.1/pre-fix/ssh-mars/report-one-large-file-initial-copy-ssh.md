# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565879189715000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `ext4 on NVMe (/dev/nvme1n1p2)`
- Transport: `ssh to sanjee@mars.local`
- Route: `ssh`
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
| rsync-a | 1.019904s | 0.057996s | 0.091726s | 100827136 | 2 | 8388608 | 0 | - | 5 | pass |
| xsync | 1.020653s | 0.144012s | 0.087002s | 73121792 | 2 | 8388608 | 8388660 | 0.876x | 5 | pass |
| xsync-rsync-transport | 1.756930s | 0.347027s | 0.105119s | 77119488 | 2 | 8388608 | 8391901 | 0.586x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 1.019904s | 0.105976s | 88670208 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 2 | warm | 1.030329s | 0.097709s | 100827136 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.638934s | 0.091726s | 42516480 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.724282s | 0.085586s | 65978368 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 2 | warm | 1.077901s | 0.091466s | 94142464 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 1 | first_pass | 1.164666s | 0.102149s | 72499200 | 2 | 8388608 | 8388660 | pass |
| xsync | 1 | 0 | warm | 0.700723s | 0.114524s | 63963136 | 2 | 8388608 | 8388660 | pass |
| xsync | 2 | 2 | warm | 1.046521s | 0.087002s | 66879488 | 2 | 8388608 | 8388660 | pass |
| xsync | 3 | 1 | warm | 1.020653s | 0.081213s | 73121792 | 2 | 8388608 | 8388660 | pass |
| xsync | 4 | 0 | warm | 0.511890s | 0.072666s | 59604992 | 2 | 8388608 | 8388660 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 2.103957s | 0.134306s | 63520768 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 1.756930s | 0.129127s | 77119488 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 2.097924s | 0.102100s | 74711040 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 0.678569s | 0.105119s | 73728000 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 0.785243s | 0.097927s | 68288512 | 2 | 8388608 | 8391901 | pass |
