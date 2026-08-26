# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568289119386000`
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
- Manifest: `dfb2d323198b7d6c41a262f66da072403a41f9e5f06d741c60cee25ef89a81be`
- Description: class=flat-small tier=smoke workload=no-op-second-sync seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.233378s | 0.021207s | 0.038264s | 7208960 | 1001 | 62000 | 0 | - | 5 | pass |
| xsync | 0.227515s | 0.009139s | 0.055926s | 7520256 | 1001 | 62000 | 0 | 1.032x | 5 | pass |
| xsync-rsync-transport | 0.380171s | 0.021422s | 0.066090s | 7274496 | 1001 | 62000 | 60084 | 0.614x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.254585s | 0.038117s | 7208960 | 1001 | 62000 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.233378s | 0.040648s | 7077888 | 1001 | 62000 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.225437s | 0.048973s | 7094272 | 1001 | 62000 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.706213s | 0.032803s | 7143424 | 1001 | 62000 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.209930s | 0.038264s | 7127040 | 1001 | 62000 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.246703s | 0.055926s | 7356416 | 1001 | 62000 | 0 | pass |
| xsync | 1 | 0 | warm | 0.227515s | 0.057142s | 7421952 | 1001 | 62000 | 0 | pass |
| xsync | 2 | 2 | warm | 0.218377s | 0.056387s | 7372800 | 1001 | 62000 | 0 | pass |
| xsync | 3 | 1 | warm | 0.214511s | 0.052734s | 7405568 | 1001 | 62000 | 0 | pass |
| xsync | 4 | 0 | warm | 0.230713s | 0.049454s | 7520256 | 1001 | 62000 | 0 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.347593s | 0.044664s | 7258112 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.380171s | 0.068744s | 7143424 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.386571s | 0.066090s | 7159808 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 0.806766s | 0.077855s | 7241728 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 0.358749s | 0.052010s | 7274496 | 1001 | 62000 | 60084 | pass |
