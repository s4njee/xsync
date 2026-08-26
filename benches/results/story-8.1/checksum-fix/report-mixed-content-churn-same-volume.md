# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787573727266566000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `804f39ed04815d2550b9146cc5ca7065` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `local`
- Route: `same-volume`
- Streams: `1`
- Compression: `none (local route)`

## Corpus

- Schema: `xsync.corpus.v1`
- Manifest: `17bfa0714453dc63deddbcd7b602cf0ed002367c36508ebbe5e730038d873880`
- Description: class=mixed tier=smoke workload=content-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.112728s | 0.007741s | 0.044884s | 5406720 | 513 | 1769997 | 0 | - | 11 | pass |
| xsync | 0.214555s | 0.017001s | 0.072862s | 16580608 | 513 | 1769997 | 0 | 0.487x | 11 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.120469s | 0.046834s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.129602s | 0.042873s | 5308416 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.150475s | 0.047052s | 5292032 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.108367s | 0.044410s | 5324800 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.100175s | 0.042603s | 5308416 | 513 | 1769997 | 0 | pass |
| rsync-a | 5 | 1 | warm | 0.112728s | 0.044974s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 6 | 0 | warm | 0.117708s | 0.050082s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 7 | 1 | warm | 0.109671s | 0.044884s | 5406720 | 513 | 1769997 | 0 | pass |
| rsync-a | 8 | 0 | warm | 0.112725s | 0.044576s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 9 | 1 | warm | 0.094153s | 0.043724s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 10 | 0 | warm | 0.125142s | 0.045730s | 5341184 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.299211s | 0.091483s | 13287424 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.293465s | 0.073127s | 15089664 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.214555s | 0.072285s | 14532608 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.235030s | 0.074376s | 16318464 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.264761s | 0.073797s | 14729216 | 513 | 1769997 | 0 | pass |
| xsync | 5 | 0 | warm | 0.198162s | 0.069299s | 15990784 | 513 | 1769997 | 0 | pass |
| xsync | 6 | 1 | warm | 0.189994s | 0.069899s | 13795328 | 513 | 1769997 | 0 | pass |
| xsync | 7 | 0 | warm | 0.202151s | 0.068682s | 16580608 | 513 | 1769997 | 0 | pass |
| xsync | 8 | 1 | warm | 0.231556s | 0.072862s | 15843328 | 513 | 1769997 | 0 | pass |
| xsync | 9 | 0 | warm | 0.204968s | 0.075347s | 12812288 | 513 | 1769997 | 0 | pass |
| xsync | 10 | 1 | warm | 0.203088s | 0.068688s | 14499840 | 513 | 1769997 | 0 | pass |
