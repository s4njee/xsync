# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568205608617000`
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
- Manifest: `b821f36f723f499539ba22d3014b4a62f6a6457149cb131dbe953dbced3baaa6`
- Description: class=mixed tier=smoke workload=type-replacement seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.070177s | 0.001882s | 0.021621s | 5111808 | 513 | 1749572 | 0 | - | 5 | pass |
| rsync-az | 0.072149s | 0.001306s | 0.021732s | 5111808 | 513 | 1749572 | 0 | 0.991x | 5 | pass |
| xsync | 0.039907s | 0.001878s | 0.028023s | 4587520 | 513 | 1749572 | 0 | 1.745x | 5 | pass |
| xsync-raw | 0.039303s | 0.000641s | 0.027418s | 4521984 | 513 | 1749572 | 0 | 1.757x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.072059s | 0.020614s | 5046272 | 513 | 1749572 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.072288s | 0.021621s | 5046272 | 513 | 1749572 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.070177s | 0.021409s | 5013504 | 513 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.049449s | 0.022669s | 5111808 | 513 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.069652s | 0.023659s | 5046272 | 513 | 1749572 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.070843s | 0.021079s | 5095424 | 513 | 1749572 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.072149s | 0.021040s | 5046272 | 513 | 1749572 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.072928s | 0.021732s | 5029888 | 513 | 1749572 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.049903s | 0.023634s | 5095424 | 513 | 1749572 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.081519s | 0.023369s | 5111808 | 513 | 1749572 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.037293s | 0.024624s | 4308992 | 513 | 1749572 | 0 | pass |
| xsync | 1 | 1 | warm | 0.038029s | 0.027017s | 4505600 | 513 | 1749572 | 0 | pass |
| xsync | 2 | 0 | warm | 0.042720s | 0.028921s | 4472832 | 513 | 1749572 | 0 | pass |
| xsync | 3 | 3 | warm | 0.040627s | 0.028560s | 4587520 | 513 | 1749572 | 0 | pass |
| xsync | 4 | 2 | warm | 0.039907s | 0.028023s | 4489216 | 513 | 1749572 | 0 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.038665s | 0.026267s | 4521984 | 513 | 1749572 | 0 | pass |
| xsync-raw | 1 | 2 | warm | 0.039303s | 0.028022s | 4472832 | 513 | 1749572 | 0 | pass |
| xsync-raw | 2 | 1 | warm | 0.039943s | 0.027639s | 4456448 | 513 | 1749572 | 0 | pass |
| xsync-raw | 3 | 0 | warm | 0.037669s | 0.026071s | 4472832 | 513 | 1749572 | 0 | pass |
| xsync-raw | 4 | 3 | warm | 0.040518s | 0.027418s | 4440064 | 513 | 1749572 | 0 | pass |
