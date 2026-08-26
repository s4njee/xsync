# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565845237907000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
| rsync-a | 0.355437s | 0.103989s | 0.051808s | 22872064 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 1.047376s | 0.491187s | 0.050809s | 7323648 | 33 | 2097152 | 2098816 | 0.615x | 5 | pass |
| xsync-rsync-transport | 1.223541s | 0.502289s | 0.062435s | 22265856 | 33 | 2097152 | 2102303 | 0.622x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.355437s | 0.051808s | 20529152 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 2 | warm | 0.251448s | 0.048139s | 22528000 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 1 | warm | 0.265766s | 0.050511s | 22839296 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 0 | warm | 1.230593s | 0.057710s | 22872064 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 2 | warm | 1.056635s | 0.059729s | 19136512 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.556190s | 0.074399s | 7323648 | 33 | 2097152 | 2098816 | pass |
| xsync | 1 | 0 | warm | 0.926390s | 0.087018s | 7274496 | 33 | 2097152 | 2098816 | pass |
| xsync | 2 | 2 | warm | 1.047376s | 0.043981s | 7225344 | 33 | 2097152 | 2098816 | pass |
| xsync | 3 | 1 | warm | 1.628822s | 0.050302s | 7274496 | 33 | 2097152 | 2098816 | pass |
| xsync | 4 | 0 | warm | 1.718221s | 0.050809s | 7290880 | 33 | 2097152 | 2098816 | pass |
| xsync-rsync-transport | 0 | 2 | first_pass | 1.725830s | 0.075017s | 19562496 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 1 | 1 | warm | 0.426291s | 0.053251s | 22265856 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 2 | 0 | warm | 0.427574s | 0.047535s | 21708800 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 3 | 2 | warm | 1.223541s | 0.062435s | 19644416 | 33 | 2097152 | 2102303 | pass |
| xsync-rsync-transport | 4 | 1 | warm | 1.297900s | 0.076587s | 21495808 | 33 | 2097152 | 2102303 | pass |
