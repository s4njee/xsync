# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568223654947000`
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
- Manifest: `904339c1374d49e04de1263354978dd256bfbe468332911b6353ced6b6b71074`
- Description: class=incompressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.078977s | 0.002804s | 0.027363s | 5193728 | 33 | 2097152 | 0 | - | 5 | pass |
| rsync-az | 0.054545s | 0.001786s | 0.029586s | 8486912 | 33 | 2097152 | 0 | 1.397x | 5 | pass |
| xsync | 0.041285s | 0.001824s | 0.027098s | 8273920 | 33 | 2097152 | 2098816 | 1.845x | 5 | pass |
| xsync-raw | 0.037766s | 0.000744s | 0.024921s | 6012928 | 33 | 2097152 | 2098816 | 2.128x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.055592s | 0.027587s | 5144576 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.078977s | 0.027248s | 5144576 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.076173s | 0.027363s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.093906s | 0.029982s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.080546s | 0.027178s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.052759s | 0.028431s | 8486912 | 33 | 2097152 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.073563s | 0.028988s | 8486912 | 33 | 2097152 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.054545s | 0.030958s | 8486912 | 33 | 2097152 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.052633s | 0.029586s | 8470528 | 33 | 2097152 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.055657s | 0.029755s | 8388608 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.043021s | 0.027098s | 7323648 | 33 | 2097152 | 2098816 | pass |
| xsync | 1 | 1 | warm | 0.043109s | 0.030774s | 8273920 | 33 | 2097152 | 2098816 | pass |
| xsync | 2 | 0 | warm | 0.041285s | 0.027891s | 7438336 | 33 | 2097152 | 2098816 | pass |
| xsync | 3 | 3 | warm | 0.038391s | 0.025389s | 6553600 | 33 | 2097152 | 2098816 | pass |
| xsync | 4 | 2 | warm | 0.037810s | 0.026468s | 7405568 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.037766s | 0.024921s | 6012928 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 1 | 2 | warm | 0.037022s | 0.024714s | 5947392 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 2 | 1 | warm | 0.035803s | 0.023351s | 5947392 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 3 | 0 | warm | 0.038262s | 0.025194s | 5931008 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 4 | 3 | warm | 0.039056s | 0.025836s | 5963776 | 33 | 2097152 | 2098816 | pass |
