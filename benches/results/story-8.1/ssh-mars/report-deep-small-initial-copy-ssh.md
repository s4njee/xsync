# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568267030193000`
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
- Manifest: `c4bbb1d39a822958eef248d6a45f442e3bb35ff00353094e853e2c88a33c6c8a`
- Description: class=deep-small tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.299267s | 0.017042s | 0.071986s | 7782400 | 1001 | 61380 | 0 | - | 5 | pass |
| xsync | 0.342920s | 0.007206s | 0.121118s | 7864320 | 1001 | 61380 | 93060 | 0.873x | 5 | pass |
| xsync-rsync-transport | 0.456782s | 0.009541s | 0.131985s | 7815168 | 1001 | 61380 | 303195 | 0.642x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.299267s | 0.064371s | 7454720 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.265575s | 0.052104s | 7487488 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.282225s | 0.071986s | 7438336 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.317148s | 0.081135s | 7454720 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.307699s | 0.085277s | 7782400 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.342920s | 0.099752s | 7864320 | 1001 | 61380 | 93060 | pass |
| xsync | 1 | 0 | warm | 0.342162s | 0.121118s | 7471104 | 1001 | 61380 | 93060 | pass |
| xsync | 2 | 2 | warm | 0.359288s | 0.126982s | 7569408 | 1001 | 61380 | 93060 | pass |
| xsync | 3 | 1 | warm | 0.327119s | 0.105499s | 7471104 | 1001 | 61380 | 93060 | pass |
| xsync | 4 | 0 | warm | 0.350126s | 0.122718s | 7520256 | 1001 | 61380 | 93060 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.466323s | 0.143916s | 7798784 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.749409s | 0.129197s | 7749632 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.443523s | 0.141311s | 7815168 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 0.456782s | 0.131985s | 7323648 | 1001 | 61380 | 303195 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 0.451644s | 0.125277s | 7536640 | 1001 | 61380 | 303195 | pass |
