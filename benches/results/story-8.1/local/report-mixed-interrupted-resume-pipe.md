# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568211341924000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `99bf54177460607ca9e072dcb5abbb73` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.1-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs (/dev/disk3s5)`
- Transport: `pipe (child xsync --server over stdio)`
- Route: `pipe`
- Streams: `1`
- Compression: `adaptive zstd`

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
| rsync-a | 0.113958s | 0.004627s | 0.073024s | 5881856 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.109721s | 0.005113s | 0.073604s | 7356416 | 513 | 1769997 | 0 | 1.016x | 5 | pass |
| xsync | 0.090511s | 0.001556s | 0.080980s | 6455296 | 513 | 1769997 | 694544 | 1.206x | 5 | pass |
| xsync-raw | 0.090181s | 0.003166s | 0.078128s | 5963776 | 513 | 1769997 | 894602 | 1.264x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.118584s | 0.074370s | 5767168 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.114809s | 0.073024s | 5849088 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.090146s | 0.074646s | 5881856 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.083915s | 0.068601s | 5849088 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.113958s | 0.071277s | 5881856 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.109721s | 0.071990s | 7340032 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.091238s | 0.076671s | 7307264 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.114834s | 0.073604s | 7356416 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.086735s | 0.069035s | 7323648 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.112144s | 0.074338s | 7323648 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.090851s | 0.081693s | 6356992 | 513 | 1769997 | 694544 | pass |
| xsync | 1 | 1 | warm | 0.087795s | 0.080441s | 6275072 | 513 | 1769997 | 694544 | pass |
| xsync | 2 | 0 | warm | 0.090511s | 0.080980s | 6373376 | 513 | 1769997 | 694544 | pass |
| xsync | 3 | 3 | warm | 0.088956s | 0.078232s | 6291456 | 513 | 1769997 | 694544 | pass |
| xsync | 4 | 2 | warm | 0.094481s | 0.081948s | 6455296 | 513 | 1769997 | 694544 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.087015s | 0.078128s | 5963776 | 513 | 1769997 | 894602 | pass |
| xsync-raw | 1 | 2 | warm | 0.089463s | 0.077720s | 5849088 | 513 | 1769997 | 894602 | pass |
| xsync-raw | 2 | 1 | warm | 0.093899s | 0.079461s | 5799936 | 513 | 1769997 | 894602 | pass |
| xsync-raw | 3 | 0 | warm | 0.095967s | 0.083931s | 5914624 | 513 | 1769997 | 894602 | pass |
| xsync-raw | 4 | 3 | warm | 0.090181s | 0.076779s | 5963776 | 513 | 1769997 | 894602 | pass |
