# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565634938805000`
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
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.082148s | 0.001739s | 0.029458s | 5226496 | 33 | 2097152 | 0 | - | 5 | pass |
| rsync-az | 0.074530s | 0.006766s | 0.026875s | 8388608 | 33 | 2097152 | 0 | 1.102x | 5 | pass |
| xsync | 0.043826s | 0.001094s | 0.028359s | 5685248 | 33 | 2097152 | 2944 | 1.872x | 5 | pass |
| xsync-raw | 0.043122s | 0.000463s | 0.027255s | 4505600 | 33 | 2097152 | 2098816 | 1.932x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.083887s | 0.029258s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.077444s | 0.029240s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.078949s | 0.030026s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.082538s | 0.029756s | 5193728 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.082148s | 0.029458s | 5160960 | 33 | 2097152 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.081296s | 0.028930s | 8388608 | 33 | 2097152 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.053035s | 0.025400s | 8372224 | 33 | 2097152 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.076666s | 0.028137s | 8388608 | 33 | 2097152 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.053243s | 0.026414s | 8306688 | 33 | 2097152 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.074530s | 0.026875s | 8290304 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.047925s | 0.027997s | 5537792 | 33 | 2097152 | 2944 | pass |
| xsync | 1 | 1 | warm | 0.041126s | 0.028468s | 5292032 | 33 | 2097152 | 2944 | pass |
| xsync | 2 | 0 | warm | 0.042732s | 0.028359s | 5685248 | 33 | 2097152 | 2944 | pass |
| xsync | 3 | 3 | warm | 0.044103s | 0.028197s | 5390336 | 33 | 2097152 | 2944 | pass |
| xsync | 4 | 2 | warm | 0.043826s | 0.028765s | 5537792 | 33 | 2097152 | 2944 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.043423s | 0.028244s | 4505600 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 1 | 2 | warm | 0.043122s | 0.027202s | 4472832 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 2 | 1 | warm | 0.043585s | 0.027578s | 4505600 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 3 | 0 | warm | 0.039596s | 0.027255s | 4210688 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 4 | 3 | warm | 0.039056s | 0.026873s | 4440064 | 33 | 2097152 | 2098816 | pass |
