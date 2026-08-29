# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787949531024476000`
- Source revision: `3b3f488dcae27c8abacdf64a7cd1d094c2af8c33-dirty`
- Build: `650960c9832d3ea9ac3742eb588b3359` (`release`)

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
- Manifest: `2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794`
- Description: real corpus=congress-100k pinned_digest=2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794

## Tools

- **xsync** `xs 0.1.0 (3b3f488dcae2 2026-08-26) aarch64-apple-darwin`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 24.024057s | 0.491290s | 31.397312s | 44285952 | 135466 | 583940018 | 24306469.465 B/s | 0 | - | 5 | pass (includes sampled) |
| xsync | 28.310084s | 0.370715s | 29.632171s | 190840832 | 135466 | 583940018 | 20626573.405 B/s | 0 | 0.842x | 5 | pass (includes sampled) |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 24.738191s | 32.187742s | 42975232 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass |
| rsync-a | 1 | 1 | warm | 24.024057s | 31.397312s | 43089920 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| rsync-a | 2 | 0 | warm | 24.255987s | 31.709419s | 37093376 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| rsync-a | 3 | 1 | warm | 23.532767s | 30.634163s | 42827776 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| rsync-a | 4 | 0 | warm | 23.373783s | 30.571745s | 44285952 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| xsync | 0 | 1 | first_pass | 28.310084s | 29.632171s | 190840832 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass |
| xsync | 1 | 0 | warm | 28.745388s | 29.988597s | 176308224 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| xsync | 2 | 1 | warm | 28.184403s | 29.343732s | 189169664 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| xsync | 3 | 0 | warm | 27.939369s | 29.432657s | 176439296 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
| xsync | 4 | 1 | warm | 29.251935s | 30.005572s | 158924800 | 135466 | 583940018 | 583940018 | 583940018 | 0 | pass (sampled) |
