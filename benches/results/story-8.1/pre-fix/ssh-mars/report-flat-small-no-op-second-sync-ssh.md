# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565776402689000`
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
- Manifest: `dfb2d323198b7d6c41a262f66da072403a41f9e5f06d741c60cee25ef89a81be`
- Description: class=flat-small tier=smoke workload=no-op-second-sync seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.657024s | 0.424000s | 0.035125s | 7192576 | 1001 | 62000 | 0 | - | 5 | pass |
| xsync | 0.236174s | 0.014962s | 0.045282s | 7897088 | 1001 | 62000 | 0 | 2.970x | 5 | pass |
| xsync-rsync-transport | 1.121635s | 0.754116s | 0.059659s | 7241728 | 1001 | 62000 | 60084 | 0.580x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.213051s | 0.035125s | 7110656 | 1001 | 62000 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.233024s | 0.048050s | 7061504 | 1001 | 62000 | 0 | pass |
| rsync-a | 2 | 1 | warm | 1.389953s | 0.038002s | 7127040 | 1001 | 62000 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.657024s | 0.029589s | 7094272 | 1001 | 62000 | 0 | pass |
| rsync-a | 4 | 2 | warm | 0.743358s | 0.031372s | 7192576 | 1001 | 62000 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.292977s | 0.056783s | 7487488 | 1001 | 62000 | 0 | pass |
| xsync | 1 | 0 | warm | 1.667885s | 0.051075s | 7372800 | 1001 | 62000 | 0 | pass |
| xsync | 2 | 2 | warm | 0.236174s | 0.045282s | 7897088 | 1001 | 62000 | 0 | pass |
| xsync | 3 | 1 | warm | 0.221212s | 0.044139s | 7421952 | 1001 | 62000 | 0 | pass |
| xsync | 4 | 0 | warm | 0.223858s | 0.036495s | 7503872 | 1001 | 62000 | 0 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.367055s | 0.067598s | 7192576 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.367519s | 0.054888s | 7176192 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 2.669268s | 0.070414s | 7241728 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 1.121635s | 0.047581s | 7176192 | 1001 | 62000 | 60084 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 1.667983s | 0.059659s | 7208960 | 1001 | 62000 | 60084 | pass |
