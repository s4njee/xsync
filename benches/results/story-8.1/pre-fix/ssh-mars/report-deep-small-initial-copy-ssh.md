# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565740076445000`
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
- Manifest: `c4bbb1d39a822958eef248d6a45f442e3bb35ff00353094e853e2c88a33c6c8a`
- Description: class=deep-small tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.794258s | 0.023066s | 0.067252s | 7487488 | 1001 | 61380 | 0 | - | 5 | pass |
| xsync | 8.730914s | 0.081425s | 0.691481s | 7028736 | 1001 | 61380 | 93928 | 0.091x | 5 | pass |
| xsync-rsync-transport | 0.940977s | 0.241598s | 0.112581s | 7733248 | 1001 | 61380 | 303195 | 0.869x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.817325s | 0.067252s | 7471104 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 2 | warm | 1.058400s | 0.063215s | 7176192 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.275781s | 0.056782s | 7487488 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.794258s | 0.071593s | 7471104 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.791040s | 0.081211s | 7454720 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 1 | first_pass | 8.562991s | 0.483549s | 7012352 | 1001 | 61380 | 93928 | pass |
| xsync | 1 | 0 | warm | 9.220565s | 0.468957s | 6995968 | 1001 | 61380 | 93928 | pass |
| xsync | 2 | 2 | warm | 8.730914s | 0.691481s | 7012352 | 1001 | 61380 | 93928 | pass |
| xsync | 3 | 1 | warm | 8.812339s | 0.771890s | 7012352 | 1001 | 61380 | 93928 | pass |
| xsync | 4 | 0 | warm | 8.658670s | 0.853631s | 7028736 | 1001 | 61380 | 93928 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.940977s | 0.108786s | 7667712 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.955980s | 0.112581s | 7733248 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.417972s | 0.110446s | 7602176 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 0.699380s | 0.115939s | 7536640 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 1.458266s | 0.138845s | 7487488 | 1001 | 61380 | 303195 | pass |
