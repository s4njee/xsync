# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568221861590000`
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
- Manifest: `22ad89d98ddb78189bbc1a1a9c1194a8b9ce8f003423c12ddd18e267f5a60897`
- Description: class=compressible tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.064148s | 0.004562s | 0.031246s | 5226496 | 33 | 2097152 | 0 | - | 5 | pass |
| rsync-az | 0.079688s | 0.000617s | 0.028213s | 8388608 | 33 | 2097152 | 0 | 0.799x | 5 | pass |
| xsync | 0.040597s | 0.002136s | 0.028123s | 6684672 | 33 | 2097152 | 2944 | 1.549x | 5 | pass |
| xsync-raw | 0.043412s | 0.004212s | 0.028142s | 6488064 | 33 | 2097152 | 2098816 | 1.636x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.064148s | 0.030814s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.080695s | 0.031246s | 5226496 | 33 | 2097152 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.085229s | 0.030918s | 5128192 | 33 | 2097152 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.059586s | 0.032390s | 5177344 | 33 | 2097152 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.061677s | 0.034860s | 5210112 | 33 | 2097152 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.080306s | 0.026907s | 8306688 | 33 | 2097152 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.078860s | 0.026858s | 8372224 | 33 | 2097152 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.079366s | 0.028213s | 8306688 | 33 | 2097152 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.079688s | 0.030037s | 8388608 | 33 | 2097152 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.084092s | 0.029448s | 8273920 | 33 | 2097152 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.044499s | 0.028123s | 6635520 | 33 | 2097152 | 2944 | pass |
| xsync | 1 | 1 | warm | 0.043383s | 0.030014s | 6684672 | 33 | 2097152 | 2944 | pass |
| xsync | 2 | 0 | warm | 0.040597s | 0.027619s | 6684672 | 33 | 2097152 | 2944 | pass |
| xsync | 3 | 3 | warm | 0.038461s | 0.026848s | 6684672 | 33 | 2097152 | 2944 | pass |
| xsync | 4 | 2 | warm | 0.040487s | 0.028159s | 6619136 | 33 | 2097152 | 2944 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.039200s | 0.027184s | 6225920 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 1 | 2 | warm | 0.043412s | 0.028142s | 6242304 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 2 | 1 | warm | 0.039015s | 0.026445s | 6176768 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 3 | 0 | warm | 0.049683s | 0.032915s | 6488064 | 33 | 2097152 | 2098816 | pass |
| xsync-raw | 4 | 3 | warm | 0.046641s | 0.030789s | 6078464 | 33 | 2097152 | 2098816 | pass |
