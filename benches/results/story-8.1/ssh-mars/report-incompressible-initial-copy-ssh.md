# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568343028173000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `ext4 on NVMe (/dev/nvme1n1p2)`
- Transport: `ssh to sanjee@mars.local`
- Route: `ssh`
- Streams: `1`
- Compression: `adaptive zstd`

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
| rsync-a | 0.474731s | 0.202281s | 0.054945s | 21921792 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.595788s | 0.154273s | 0.061424s | 23396352 | 33 | 2097152 | 2098816 | 1.089x | 5 | pass |
| xsync-rsync-transport | 0.911819s | 0.073147s | 0.077392s | 24395776 | 33 | 2097152 | 2102303 | 0.645x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.474731s | 0.051345s | 16187392 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.817095s | 0.056088s | 21397504 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.272450s | 0.075700s | 18825216 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 0 | warm | 0.300316s | 0.054945s | 21921792 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 2 | warm | 1.313434s | 0.049725s | 20578304 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.311206s | 0.068263s | 21512192 | 33 | 2097152 | 2098816 | pass |
| xsync | 1 | 0 | warm | 0.750061s | 0.051463s | 19562496 | 33 | 2097152 | 2098816 | pass |
| xsync | 2 | 2 | warm | 0.595788s | 0.061424s | 22593536 | 33 | 2097152 | 2098816 | pass |
| xsync | 3 | 1 | warm | 0.282034s | 0.061516s | 21889024 | 33 | 2097152 | 2098816 | pass |
| xsync | 4 | 0 | warm | 0.723818s | 0.050441s | 23396352 | 33 | 2097152 | 2098816 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 0.917745s | 0.087926s | 21315584 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.984966s | 0.077392s | 24395776 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.422385s | 0.093020s | 22118400 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 0.911819s | 0.074013s | 18169856 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 0.663228s | 0.061260s | 18726912 | 33 | 2097152 | 2102303 | pass |
