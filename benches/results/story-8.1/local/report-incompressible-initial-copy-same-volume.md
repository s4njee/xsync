# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568111225458000`
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
- Manifest: `904339c1374d49e04de1263354978dd256bfbe468332911b6353ced6b6b71074`
- Description: class=incompressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.043383s | 0.001399s | 0.020242s | 5210112 | 33 | 2097152 | 0 | - | 5 | pass |
| xsync | 0.032284s | 0.000174s | 0.099347s | 4177920 | 33 | 2097152 | 0 | 1.396x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.043566s | 0.019854s | 5111808 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 1 | warm | 0.045748s | 0.021273s | 5144576 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 0 | warm | 0.041560s | 0.020000s | 5079040 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.043383s | 0.020242s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.041984s | 0.020563s | 5210112 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 1 | first_pass | 0.032359s | 0.104277s | 4079616 | 33 | 2097152 | 0 | pass |
| xsync | 1 | 0 | warm | 0.032458s | 0.099347s | 4079616 | 33 | 2097152 | 0 | pass |
| xsync | 2 | 1 | warm | 0.029550s | 0.101036s | 4063232 | 33 | 2097152 | 0 | pass |
| xsync | 3 | 0 | warm | 0.032284s | 0.084650s | 4177920 | 33 | 2097152 | 0 | pass |
| xsync | 4 | 1 | warm | 0.030078s | 0.086431s | 4161536 | 33 | 2097152 | 0 | pass |
