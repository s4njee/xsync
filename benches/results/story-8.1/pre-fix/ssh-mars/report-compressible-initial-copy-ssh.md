# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565807250897000`
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
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.304343s | 0.019372s | 0.068077s | 22855680 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 1.040802s | 0.009819s | 0.076053s | 7110656 | 33 | 2097152 | 2944 | 0.364x | 5 | pass |
| xsync-rsync-transport | 0.955239s | 0.141617s | 0.095656s | 22429696 | 33 | 2097152 | 2102303 | 0.307x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.304343s | 0.068077s | 18841600 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.293643s | 0.065664s | 20348928 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.382041s | 0.073407s | 19202048 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.779357s | 0.065918s | 22855680 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.284971s | 0.068206s | 18956288 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 1.452582s | 0.085085s | 7045120 | 33 | 2097152 | 2944 | pass |
| xsync | 1 | 0 | warm | 1.033784s | 0.076053s | 7045120 | 33 | 2097152 | 2944 | pass |
| xsync | 2 | 2 | warm | 1.050621s | 0.082094s | 7028736 | 33 | 2097152 | 2944 | pass |
| xsync | 3 | 1 | warm | 1.040802s | 0.072677s | 7110656 | 33 | 2097152 | 2944 | pass |
| xsync | 4 | 0 | warm | 0.521729s | 0.066620s | 6995968 | 33 | 2097152 | 2944 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 1.096855s | 0.069295s | 15810560 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.955239s | 0.097689s | 19202048 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 1.950590s | 0.098440s | 22429696 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 0.942270s | 0.094810s | 22413312 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 0.456667s | 0.095656s | 18907136 | 33 | 2097152 | 2102303 | pass |
