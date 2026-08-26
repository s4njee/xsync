# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568199139017000`
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
- Manifest: `986714ad0b4ccffcef8392c8fbbcd47fc542a83c383184beaa7a35dfcccba26e`
- Description: class=mixed tier=smoke workload=delete seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.071007s | 0.004449s | 0.022529s | 5095424 | 509 | 1749572 | 0 | - | 5 | pass |
| rsync-az | 0.070576s | 0.001994s | 0.022063s | 5111808 | 509 | 1749572 | 0 | 0.957x | 5 | pass |
| xsync | 0.039335s | 0.001815s | 0.026100s | 4505600 | 509 | 1749572 | 0 | 1.300x | 5 | pass |
| xsync-raw | 0.038539s | 0.000637s | 0.026911s | 4571136 | 509 | 1749572 | 0 | 1.806x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.051131s | 0.022628s | 5079040 | 509 | 1749572 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.048050s | 0.021693s | 5079040 | 509 | 1749572 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.071007s | 0.022529s | 5079040 | 509 | 1749572 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.075456s | 0.022380s | 5095424 | 509 | 1749572 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.072346s | 0.022773s | 5062656 | 509 | 1749572 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.070557s | 0.022716s | 5095424 | 509 | 1749572 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.072570s | 0.022063s | 5111808 | 509 | 1749572 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.070576s | 0.021936s | 5095424 | 509 | 1749572 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.068526s | 0.021448s | 5111808 | 509 | 1749572 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.075615s | 0.022182s | 5079040 | 509 | 1749572 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.039335s | 0.026100s | 4407296 | 509 | 1749572 | 0 | pass |
| xsync | 1 | 1 | warm | 0.041150s | 0.028647s | 4456448 | 509 | 1749572 | 0 | pass |
| xsync | 2 | 0 | warm | 0.036025s | 0.023474s | 4358144 | 509 | 1749572 | 0 | pass |
| xsync | 3 | 3 | warm | 0.059722s | 0.031248s | 4440064 | 509 | 1749572 | 0 | pass |
| xsync | 4 | 2 | warm | 0.037737s | 0.025723s | 4505600 | 509 | 1749572 | 0 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.043052s | 0.026167s | 4472832 | 509 | 1749572 | 0 | pass |
| xsync-raw | 1 | 2 | warm | 0.038539s | 0.027263s | 4571136 | 509 | 1749572 | 0 | pass |
| xsync-raw | 2 | 1 | warm | 0.037941s | 0.027042s | 4571136 | 509 | 1749572 | 0 | pass |
| xsync-raw | 3 | 0 | warm | 0.041792s | 0.026911s | 4390912 | 509 | 1749572 | 0 | pass |
| xsync-raw | 4 | 3 | warm | 0.037902s | 0.025950s | 4505600 | 509 | 1749572 | 0 | pass |
