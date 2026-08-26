# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565589920223000`
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
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.044963s | 0.000390s | 0.022810s | 5226496 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.033082s | 0.000107s | 0.096973s | 4227072 | 33 | 2097152 | 0 | 1.350x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.047578s | 0.023700s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.046631s | 0.022788s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.044573s | 0.023053s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.044595s | 0.022810s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.044963s | 0.022755s | 5160960 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.035073s | 0.099384s | 4079616 | 33 | 2097152 | 0 | pass |
| xsync | 1 | 0 | warm | 0.034787s | 0.096533s | 4227072 | 33 | 2097152 | 0 | pass |
| xsync | 2 | 1 | warm | 0.033082s | 0.096973s | 4046848 | 33 | 2097152 | 0 | pass |
| xsync | 3 | 0 | warm | 0.033041s | 0.092530s | 4096000 | 33 | 2097152 | 0 | pass |
| xsync | 4 | 1 | warm | 0.032975s | 0.099108s | 4112384 | 33 | 2097152 | 0 | pass |
