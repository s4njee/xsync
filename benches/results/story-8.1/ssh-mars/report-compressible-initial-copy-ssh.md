# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568315061188000`
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
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.795704s | 0.001829s | 0.064709s | 23625728 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.727729s | 0.158544s | 0.039432s | 7585792 | 33 | 2097152 | 2944 | 1.096x | 5 | pass |
| xsync-rsync-transport | 0.964164s | 0.483404s | 0.076026s | 22347776 | 33 | 2097152 | 2102303 | 0.609x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.797381s | 0.072350s | 20381696 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.774045s | 0.061780s | 16875520 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.264236s | 0.059077s | 23625728 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.795704s | 0.064709s | 22659072 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.797532s | 0.084817s | 22003712 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.727729s | 0.039432s | 7307264 | 33 | 2097152 | 2944 | pass |
| xsync | 1 | 0 | warm | 0.197927s | 0.038492s | 7585792 | 33 | 2097152 | 2944 | pass |
| xsync | 2 | 2 | warm | 0.224914s | 0.041962s | 7159808 | 33 | 2097152 | 2944 | pass |
| xsync | 3 | 1 | warm | 0.886273s | 0.054817s | 7094272 | 33 | 2097152 | 2944 | pass |
| xsync | 4 | 0 | warm | 0.736273s | 0.034910s | 7421952 | 33 | 2097152 | 2944 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.964164s | 0.076026s | 18137088 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.890668s | 0.076064s | 20316160 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.433779s | 0.092307s | 22347776 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 1.451601s | 0.067812s | 22298624 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 1.447568s | 0.065758s | 19759104 | 33 | 2097152 | 2102303 | pass |
