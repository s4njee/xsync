# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568088979740000`
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
- Manifest: `17bfa0714453dc63deddbcd7b602cf0ed002367c36508ebbe5e730038d873880`
- Description: class=mixed tier=smoke workload=content-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.064452s | 0.000539s | 0.033843s | 5324800 | 513 | 1769997 | 0 | - | 5 | pass |
| xsync | 4.088821s | 0.222135s | 0.413967s | 15548416 | 513 | 1769997 | 0 | 0.016x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.064019s | 0.032169s | 5275648 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.065529s | 0.034573s | 5275648 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.088287s | 0.034654s | 5324800 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.064452s | 0.033715s | 5275648 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.063913s | 0.033843s | 5242880 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 1 | first_pass | 8.220459s | 0.710450s | 14974976 | 513 | 1769997 | 0 | pass |
| xsync | 1 | 0 | warm | 4.803827s | 0.405501s | 15171584 | 513 | 1769997 | 0 | pass |
| xsync | 2 | 1 | warm | 3.866686s | 0.413967s | 15384576 | 513 | 1769997 | 0 | pass |
| xsync | 3 | 0 | warm | 3.942268s | 0.441426s | 15073280 | 513 | 1769997 | 0 | pass |
| xsync | 4 | 1 | warm | 4.088821s | 0.389619s | 15548416 | 513 | 1769997 | 0 | pass |
