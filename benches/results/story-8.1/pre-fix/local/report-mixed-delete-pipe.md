# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565623719711000`
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
- Manifest: `986714ad0b4ccffcef8392c8fbbcd47fc542a83c383184beaa7a35dfcccba26e`
- Description: class=mixed tier=smoke workload=delete seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.074992s | 0.002897s | 0.024085s | 5210112 | 509 | 1749572 | 0 | - | 5 | pass |
| rsync-az | 0.068460s | 0.009429s | 0.023564s | 5226496 | 509 | 1749572 | 0 | 0.963x | 5 | pass |
| xsync | 0.041250s | 0.001382s | 0.026744s | 4669440 | 509 | 1749572 | 0 | 1.678x | 5 | pass |
| xsync-raw | 0.040853s | 0.001642s | 0.026738s | 4653056 | 509 | 1749572 | 0 | 1.601x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.077889s | 0.023272s | 5177344 | 509 | 1749572 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.050976s | 0.025001s | 5144576 | 509 | 1749572 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.077514s | 0.023939s | 5177344 | 509 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.056578s | 0.025143s | 5210112 | 509 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.074992s | 0.024085s | 5160960 | 509 | 1749572 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.053940s | 0.023564s | 5128192 | 509 | 1749572 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.068460s | 0.022836s | 5095424 | 509 | 1749572 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.049277s | 0.023126s | 5177344 | 509 | 1749572 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.077464s | 0.024358s | 5210112 | 509 | 1749572 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.077889s | 0.024848s | 5226496 | 509 | 1749572 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.046418s | 0.028734s | 4505600 | 509 | 1749572 | 0 | pass |
| xsync | 1 | 1 | warm | 0.040689s | 0.029055s | 4620288 | 509 | 1749572 | 0 | pass |
| xsync | 2 | 0 | warm | 0.039423s | 0.026744s | 4571136 | 509 | 1749572 | 0 | pass |
| xsync | 3 | 3 | warm | 0.041250s | 0.026703s | 4669440 | 509 | 1749572 | 0 | pass |
| xsync | 4 | 2 | warm | 0.042632s | 0.026262s | 4620288 | 509 | 1749572 | 0 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.038046s | 0.027572s | 4603904 | 509 | 1749572 | 0 | pass |
| xsync-raw | 1 | 2 | warm | 0.042495s | 0.026541s | 4505600 | 509 | 1749572 | 0 | pass |
| xsync-raw | 2 | 1 | warm | 0.040476s | 0.026738s | 4603904 | 509 | 1749572 | 0 | pass |
| xsync-raw | 3 | 0 | warm | 0.040853s | 0.026146s | 4636672 | 509 | 1749572 | 0 | pass |
| xsync-raw | 4 | 3 | warm | 0.046833s | 0.030363s | 4653056 | 509 | 1749572 | 0 | pass |
