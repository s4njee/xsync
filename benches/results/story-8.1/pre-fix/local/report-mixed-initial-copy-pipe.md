# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565596768090000`
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
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.151745s | 0.014973s | 0.125408s | 5406720 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.156506s | 0.001970s | 0.126853s | 8339456 | 513 | 1769997 | 0 | 0.985x | 5 | pass |
| xsync | 0.165973s | 0.000921s | 0.153924s | 5701632 | 513 | 1769997 | 915896 | 0.914x | 5 | pass |
| xsync-raw | 0.163648s | 0.002905s | 0.150405s | 5242880 | 513 | 1769997 | 1794073 | 0.894x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.136772s | 0.118292s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.154837s | 0.125408s | 5406720 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.194456s | 0.134198s | 5406720 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.151745s | 0.124717s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.132260s | 0.125568s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.158475s | 0.126853s | 8257536 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.134067s | 0.125373s | 8339456 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.158394s | 0.130954s | 8273920 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.154014s | 0.124029s | 8273920 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.156506s | 0.127307s | 8323072 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.170944s | 0.155034s | 5652480 | 513 | 1769997 | 915896 | pass |
| xsync | 1 | 1 | warm | 0.165420s | 0.157379s | 5701632 | 513 | 1769997 | 915896 | pass |
| xsync | 2 | 0 | warm | 0.165052s | 0.153023s | 5636096 | 513 | 1769997 | 915896 | pass |
| xsync | 3 | 3 | warm | 0.165973s | 0.153924s | 5652480 | 513 | 1769997 | 915896 | pass |
| xsync | 4 | 2 | warm | 0.167418s | 0.151974s | 5636096 | 513 | 1769997 | 915896 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.166553s | 0.151533s | 4947968 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 1 | 2 | warm | 0.162254s | 0.147807s | 5242880 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 2 | 1 | warm | 0.163648s | 0.150125s | 5013504 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 3 | 0 | warm | 0.169798s | 0.150405s | 5177344 | 513 | 1769997 | 1794073 | pass |
| xsync-raw | 4 | 3 | warm | 0.157484s | 0.151508s | 5095424 | 513 | 1769997 | 1794073 | pass |
