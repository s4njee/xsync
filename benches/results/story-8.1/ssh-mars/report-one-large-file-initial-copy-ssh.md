# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568367736841000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

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
| rsync-a | 0.881976s | 0.058759s | 0.085359s | 128122880 | 2 | 8388608 | 0 | - | 5 | pass |
| xsync | 0.878301s | 0.056525s | 0.079074s | 100483072 | 2 | 8388608 | 8388660 | 1.030x | 5 | pass |
| xsync-rsync-transport | 1.035650s | 0.379260s | 0.107073s | 92372992 | 2 | 8388608 | 8391901 | 0.645x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.871464s | 0.082080s | 99647488 | 2 | 8388608 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.940736s | 0.083557s | 108576768 | 2 | 8388608 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.546309s | 0.086291s | 128122880 | 2 | 8388608 | 0 | pass |
| rsync-a | 3 | 0 | warm | 1.005713s | 0.085359s | 122257408 | 2 | 8388608 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.881976s | 0.087243s | 102465536 | 2 | 8388608 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.878301s | 0.074168s | 83574784 | 2 | 8388608 | 8388660 | pass |
| xsync | 1 | 0 | warm | 0.378414s | 0.077211s | 97583104 | 2 | 8388608 | 8388660 | pass |
| xsync | 2 | 2 | warm | 1.528583s | 0.079701s | 82886656 | 2 | 8388608 | 8388660 | pass |
| xsync | 3 | 1 | warm | 0.934825s | 0.079074s | 77627392 | 2 | 8388608 | 8388660 | pass |
| xsync | 4 | 0 | warm | 0.855910s | 0.079825s | 100483072 | 2 | 8388608 | 8388660 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.656390s | 0.099945s | 72810496 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 1.459128s | 0.107073s | 74055680 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.899675s | 0.111136s | 89030656 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 1.035650s | 0.102711s | 88981504 | 2 | 8388608 | 8391901 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 1.541282s | 0.108230s | 92372992 | 2 | 8388608 | 8391901 | pass |
