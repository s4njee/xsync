# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568096539057000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

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
- Manifest: `986714ad0b4ccffcef8392c8fbbcd47fc542a83c383184beaa7a35dfcccba26e`
- Description: class=mixed tier=smoke workload=delete seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.039621s | 0.000072s | 0.016173s | 5095424 | 509 | 1749572 | 0 | - | 5 | pass |
| xsync | 0.034890s | 0.003108s | 0.020755s | 5292032 | 509 | 1749572 | 0 | 1.107x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.042670s | 0.017480s | 5095424 | 509 | 1749572 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.039691s | 0.016045s | 5062656 | 509 | 1749572 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.038627s | 0.016834s | 5079040 | 509 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.039621s | 0.016173s | 5062656 | 509 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.039549s | 0.015717s | 5046272 | 509 | 1749572 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.034545s | 0.020529s | 5275648 | 509 | 1749572 | 0 | pass |
| xsync | 1 | 0 | warm | 0.038315s | 0.020755s | 5160960 | 509 | 1749572 | 0 | pass |
| xsync | 2 | 1 | warm | 0.034890s | 0.024002s | 5210112 | 509 | 1749572 | 0 | pass |
| xsync | 3 | 0 | warm | 0.029262s | 0.018073s | 5046272 | 509 | 1749572 | 0 | pass |
| xsync | 4 | 1 | warm | 0.037998s | 0.020800s | 5292032 | 509 | 1749572 | 0 | pass |
