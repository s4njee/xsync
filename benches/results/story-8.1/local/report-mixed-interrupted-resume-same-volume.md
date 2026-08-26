# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568103650404000`
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
- Manifest: `7495ac984b2d8c9a9aef7234e0ccec6b5568865c63420e350d1888c611575db6`
- Description: class=mixed tier=smoke workload=interrupted-resume seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.080773s | 0.000144s | 0.069854s | 5373952 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 0.118765s | 0.003471s | 0.582746s | 5505024 | 513 | 1769997 | 0 | 0.680x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.081219s | 0.063353s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.080773s | 0.070464s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.108714s | 0.069854s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.080629s | 0.067723s | 5373952 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.080682s | 0.070017s | 5373952 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.115293s | 0.604640s | 5455872 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 0.118765s | 0.575087s | 5160960 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 0.117851s | 0.582746s | 5292032 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 0.126959s | 0.592995s | 5505024 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 0.147103s | 0.573502s | 5324800 | 513 | 1769997 | 0 | pass |
