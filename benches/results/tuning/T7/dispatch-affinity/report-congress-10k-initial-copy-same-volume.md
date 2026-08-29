# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787925714522905000`
- Source revision: `3b3f488dcae27c8abacdf64a7cd1d094c2af8c33-dirty`
- Build: `c47c070cfffd4d4b4fbc81e149b0de7c` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `local`
- Route: `same-volume`
- Shaping: `none`
- Streams: `1`
- Compression: `none (local route)`

## Corpus

- Schema: `xsync.manifest.v1`
- Manifest: `f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`
- Description: real corpus=congress-10k pinned_digest=f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 2.822868s | 0.007314s | 4.447658s | 53395456 | 22568 | 96542108 | 34200012.988 B/s | 0 | - | 5 | pass |
| xsync | 5.286046s | 0.023613s | 6.382648s | 42385408 | 22568 | 96542108 | 18263576.827 B/s | 0 | 0.534x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 2.822868s | 4.440012s | 48578560 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 2.815554s | 4.447658s | 47464448 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 2 | 0 | warm | 2.820782s | 4.447478s | 46678016 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 3 | 1 | warm | 2.854600s | 4.580740s | 48087040 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 4 | 0 | warm | 2.942315s | 4.725068s | 53395456 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 0 | 1 | first_pass | 5.262433s | 6.461937s | 34521088 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 1 | 0 | warm | 5.333586s | 6.382648s | 41730048 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 2 | 1 | warm | 5.278119s | 6.285413s | 42385408 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 3 | 0 | warm | 5.286046s | 6.444697s | 41861120 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| xsync | 4 | 1 | warm | 5.523041s | 6.282069s | 36405248 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
