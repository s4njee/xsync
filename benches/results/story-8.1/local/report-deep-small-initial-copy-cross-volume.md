# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568166057629000`
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
- Manifest: `c4bbb1d39a822958eef248d6a45f442e3bb35ff00353094e853e2c88a33c6c8a`
- Description: class=deep-small tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.258338s | 0.010027s | 0.237105s | 5259264 | 1001 | 61380 | 0 | - | 5 | pass |
| xsync | 0.455803s | 0.037069s | 2.747783s | 8093696 | 1001 | 61380 | 0 | 0.561x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.248311s | 0.234538s | 5259264 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.275567s | 0.265243s | 5226496 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.258338s | 0.252432s | 5160960 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.241543s | 0.234435s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.267929s | 0.237105s | 5177344 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.583993s | 2.694582s | 8060928 | 1001 | 61380 | 0 | pass |
| xsync | 1 | 0 | warm | 0.528870s | 2.759833s | 7897088 | 1001 | 61380 | 0 | pass |
| xsync | 2 | 1 | warm | 0.455803s | 2.775383s | 7995392 | 1001 | 61380 | 0 | pass |
| xsync | 3 | 0 | warm | 0.430759s | 2.747783s | 8093696 | 1001 | 61380 | 0 | pass |
| xsync | 4 | 1 | warm | 0.418734s | 2.648749s | 7847936 | 1001 | 61380 | 0 | pass |
