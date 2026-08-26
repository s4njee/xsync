# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565588997020000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.251899s | 0.005154s | 0.241781s | 5242880 | 1001 | 61380 | 0 | - | 5 | pass |
| xsync | 0.494902s | 0.004585s | 2.740649s | 7929856 | 1001 | 61380 | 0 | 0.517x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.246745s | 0.235330s | 5193728 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.255481s | 0.246640s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.251899s | 0.245249s | 5242880 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.275650s | 0.241781s | 5242880 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.243639s | 0.236455s | 5193728 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.494902s | 2.704538s | 7929856 | 1001 | 61380 | 0 | pass |
| xsync | 1 | 0 | warm | 0.493704s | 2.785831s | 7847936 | 1001 | 61380 | 0 | pass |
| xsync | 2 | 1 | warm | 0.485936s | 2.747678s | 7733248 | 1001 | 61380 | 0 | pass |
| xsync | 3 | 0 | warm | 0.507628s | 2.740649s | 7602176 | 1001 | 61380 | 0 | pass |
| xsync | 4 | 1 | warm | 0.499487s | 2.739363s | 7651328 | 1001 | 61380 | 0 | pass |
