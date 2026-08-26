# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568173412277000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

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
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.155938s | 0.004540s | 0.123234s | 5423104 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.149563s | 0.002279s | 0.117818s | 8306688 | 513 | 1769997 | 0 | 1.027x | 5 | pass |
| xsync | 0.138775s | 0.001417s | 0.132240s | 7667712 | 513 | 1769997 | 915733 | 1.099x | 5 | pass |
| xsync-raw | 0.147348s | 0.003564s | 0.134135s | 7110656 | 513 | 1769997 | 1794073 | 1.029x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.160477s | 0.125153s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.158818s | 0.133411s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.155938s | 0.123234s | 5423104 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.127290s | 0.115982s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.147678s | 0.115792s | 5423104 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.148048s | 0.117818s | 8273920 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.154388s | 0.119791s | 8241152 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.151842s | 0.119338s | 8306688 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.145602s | 0.115328s | 8224768 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.149563s | 0.115890s | 8241152 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.139802s | 0.133125s | 7553024 | 513 | 1769997 | 915733 | pass |
| xsync | 1 | 1 | warm | 0.135554s | 0.128596s | 7340032 | 513 | 1769997 | 915733 | pass |
| xsync | 2 | 0 | warm | 0.141840s | 0.135403s | 7667712 | 513 | 1769997 | 915733 | pass |
| xsync | 3 | 3 | warm | 0.138775s | 0.132240s | 7618560 | 513 | 1769997 | 915733 | pass |
| xsync | 4 | 2 | warm | 0.137359s | 0.127745s | 7536640 | 513 | 1769997 | 915733 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.147976s | 0.134135s | 6799360 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 1 | 2 | warm | 0.156169s | 0.138965s | 6946816 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 2 | 1 | warm | 0.147348s | 0.139703s | 6979584 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 3 | 0 | warm | 0.143783s | 0.131640s | 6963200 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 4 | 3 | warm | 0.143524s | 0.132269s | 7110656 | 513 | 1769997 | 1794073 | pass |
