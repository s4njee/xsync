# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565565458947000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk11s1)`
- Transport: `local`
- Route: `cross-volume`
- Streams: `1`
- Compression: `none (local route)`

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
| rsync-a | 0.129061s | 0.001706s | 0.125471s | 5423104 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.235967s | 0.003395s | 1.246073s | 5128192 | 513 | 1769997 | 0 | 0.555x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.128672s | 0.121381s | 5390336 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.132430s | 0.127568s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.123823s | 0.121098s | 5357568 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.129061s | 0.125471s | 5423104 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.130768s | 0.126623s | 5406720 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.236449s | 1.245506s | 5062656 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.235967s | 1.211088s | 5111808 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.244843s | 1.250326s | 5029888 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.232572s | 1.264579s | 5029888 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.227850s | 1.246073s | 5128192 | 513 | 1769997 | 0 | pass |
