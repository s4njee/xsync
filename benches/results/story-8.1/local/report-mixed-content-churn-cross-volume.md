# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568144224735000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

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
- Manifest: `17bfa0714453dc63deddbcd7b602cf0ed002367c36508ebbe5e730038d873880`
- Description: class=mixed tier=smoke workload=content-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.119798s | 0.004230s | 0.044760s | 5341184 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 3.781377s | 0.157272s | 0.312799s | 15450112 | 513 | 1769997 | 0 | 0.032x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.116675s | 0.041023s | 5324800 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.126636s | 0.050948s | 5341184 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.119798s | 0.044760s | 5324800 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.124224s | 0.047581s | 5324800 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.115569s | 0.043347s | 5324800 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 7.475392s | 0.631362s | 12419072 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 3.927865s | 0.291893s | 15450112 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 3.113233s | 0.274242s | 13991936 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 3.624105s | 0.312799s | 15155200 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 3.781377s | 0.314167s | 15417344 | 513 | 1769997 | 0 | pass |
