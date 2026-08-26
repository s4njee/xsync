# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568186335064000`
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
- Manifest: `17bfa0714453dc63deddbcd7b602cf0ed002367c36508ebbe5e730038d873880`
- Description: class=mixed tier=smoke workload=content-churn seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.095917s | 0.005126s | 0.042605s | 5619712 | 513 | 1769997 | 0 | - | 5 | pass |
| rsync-az | 0.101161s | 0.002440s | 0.042839s | 6733824 | 513 | 1769997 | 0 | 0.968x | 5 | pass |
| xsync | 0.066429s | 0.000265s | 0.052047s | 4931584 | 513 | 1769997 | 4131 | 1.472x | 5 | pass |
| xsync-raw | 0.068538s | 0.000540s | 0.053304s | 4751360 | 513 | 1769997 | 20633 | 1.434x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.095530s | 0.038783s | 5603328 | 513 | 1769997 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.102044s | 0.042605s | 5570560 | 513 | 1769997 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.095917s | 0.041510s | 5554176 | 513 | 1769997 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.101043s | 0.042820s | 5619712 | 513 | 1769997 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.086991s | 0.045361s | 5619712 | 513 | 1769997 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.098721s | 0.042679s | 6635520 | 513 | 1769997 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.094395s | 0.042600s | 6668288 | 513 | 1769997 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.101161s | 0.042839s | 6684672 | 513 | 1769997 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.104390s | 0.042982s | 6668288 | 513 | 1769997 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.103310s | 0.044818s | 6733824 | 513 | 1769997 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.066429s | 0.049953s | 4882432 | 513 | 1769997 | 4131 | pass |
| xsync | 1 | 1 | warm | 0.066229s | 0.054361s | 4866048 | 513 | 1769997 | 4131 | pass |
| xsync | 2 | 0 | warm | 0.065155s | 0.051342s | 4800512 | 513 | 1769997 | 4131 | pass |
| xsync | 3 | 3 | warm | 0.066693s | 0.052047s | 4849664 | 513 | 1769997 | 4131 | pass |
| xsync | 4 | 2 | warm | 0.072480s | 0.055377s | 4931584 | 513 | 1769997 | 4131 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.067487s | 0.051092s | 4653056 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 1 | 2 | warm | 0.069077s | 0.053304s | 4718592 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 2 | 1 | warm | 0.066871s | 0.055437s | 4734976 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 3 | 0 | warm | 0.068705s | 0.047087s | 4390912 | 513 | 1769997 | 20633 | pass |
| xsync-raw | 4 | 3 | warm | 0.068538s | 0.054673s | 4751360 | 513 | 1769997 | 20633 | pass |
