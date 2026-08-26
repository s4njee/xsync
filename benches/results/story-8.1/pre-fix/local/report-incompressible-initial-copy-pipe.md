# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565636893112000`
- Source revision: `f5e10179c9590e193265a5001965d3ad985003b0-dirty`
- Build: `be86b28917e1ef8f64dede9c2f91c3a9` (`release`)

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
- Manifest: `904339c1374d49e04de1263354978dd256bfbe468332911b6353ced6b6b71074`
- Description: class=incompressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.080747s | 0.002124s | 0.033735s | 5259264 | 33 | 2097152 | 0 | - | 5 | pass |
| rsync-az | 0.060607s | 0.005604s | 0.034134s | 8568832 | 33 | 2097152 | 0 | 1.094x | 5 | pass |
| xsync | 0.049679s | 0.000878s | 0.031008s | 5701632 | 33 | 2097152 | 2098816 | 1.639x | 5 | pass |
| xsync-raw | 0.044627s | 0.002803s | 0.027847s | 4554752 | 33 | 2097152 | 2098816 | 1.763x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.078694s | 0.028742s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.082871s | 0.033735s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.086497s | 0.037723s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.080747s | 0.032735s | 5259264 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.060153s | 0.034431s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.087802s | 0.031965s | 8454144 | 33 | 2097152 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.060607s | 0.034134s | 8486912 | 33 | 2097152 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.082307s | 0.034437s | 8486912 | 33 | 2097152 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.059528s | 0.034612s | 8503296 | 33 | 2097152 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.055003s | 0.031151s | 8568832 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.050179s | 0.031008s | 5603328 | 33 | 2097152 | 2098816 | pass |
| xsync | 1 | 1 | warm | 0.050557s | 0.033774s | 5652480 | 33 | 2097152 | 2098816 | pass |
| xsync | 2 | 0 | warm | 0.049679s | 0.033449s | 5701632 | 33 | 2097152 | 2098816 | pass |
| xsync | 3 | 3 | warm | 0.044658s | 0.030334s | 5668864 | 33 | 2097152 | 2098816 | pass |
| xsync | 4 | 2 | warm | 0.044354s | 0.028406s | 5668864 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.044627s | 0.027600s | 4358144 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 1 | 2 | warm | 0.047431s | 0.029683s | 4292608 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 2 | 1 | warm | 0.048501s | 0.031107s | 4554752 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 3 | 0 | warm | 0.043095s | 0.027609s | 4472832 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 4 | 3 | warm | 0.038622s | 0.027847s | 4440064 | 33 | 2097152 | 2098816 | pass |
