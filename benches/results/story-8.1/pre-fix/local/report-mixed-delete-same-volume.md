# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565553462125000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.038562s | 0.000994s | 0.016347s | 5095424 | 509 | 1749572 | 0 | - | 5 | pass |
| xsync | 0.027491s | 0.000799s | 0.021265s | 5341184 | 509 | 1749572 | 0 | 1.479x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.060166s | 0.016009s | 5062656 | 509 | 1749572 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.037568s | 0.016483s | 5079040 | 509 | 1749572 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.038562s | 0.016516s | 5095424 | 509 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.037740s | 0.015976s | 5079040 | 509 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.060764s | 0.016347s | 5079040 | 509 | 1749572 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.027491s | 0.022326s | 5341184 | 509 | 1749572 | 0 | pass |
| xsync | 1 | 0 | warm | 0.028290s | 0.021265s | 5144576 | 509 | 1749572 | 0 | pass |
| xsync | 2 | 1 | warm | 0.027653s | 0.020345s | 5095424 | 509 | 1749572 | 0 | pass |
| xsync | 3 | 0 | warm | 0.025517s | 0.021387s | 5111808 | 509 | 1749572 | 0 | pass |
| xsync | 4 | 1 | warm | 0.025926s | 0.019218s | 5029888 | 509 | 1749572 | 0 | pass |
