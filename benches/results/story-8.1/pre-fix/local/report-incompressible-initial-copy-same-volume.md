# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565561325130000`
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
- Manifest: `904339c1374d49e04de1263354978dd256bfbe468332911b6353ced6b6b71074`
- Description: class=incompressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.046187s | 0.000685s | 0.023315s | 5210112 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.032851s | 0.002220s | 0.086880s | 4194304 | 33 | 2097152 | 0 | 1.370x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.046187s | 0.023775s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.046436s | 0.023315s | 5210112 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.070116s | 0.024632s | 5210112 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.045503s | 0.023123s | 5210112 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.043231s | 0.021733s | 5210112 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.039425s | 0.083935s | 4128768 | 33 | 2097152 | 0 | pass |
| xsync | 1 | 0 | warm | 0.037836s | 0.088984s | 4046848 | 33 | 2097152 | 0 | pass |
| xsync | 2 | 1 | warm | 0.032851s | 0.097184s | 4046848 | 33 | 2097152 | 0 | pass |
| xsync | 3 | 0 | warm | 0.030631s | 0.086880s | 4194304 | 33 | 2097152 | 0 | pass |
| xsync | 4 | 1 | warm | 0.031552s | 0.083617s | 4063232 | 33 | 2097152 | 0 | pass |
