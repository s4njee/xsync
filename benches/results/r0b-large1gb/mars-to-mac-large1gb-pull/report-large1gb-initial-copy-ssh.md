# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1788212970042747000`
- Source revision: `943b8ec49bc2f3aa0f9a503598b86d3d68f24f57-dirty`
- Build: `4b89da9122c4b5a19f385a6e63d098bf` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.2-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `apfs`
- Transport: `ssh to sanjee@mars.local`
- Route: `mars-to-mbp-router1-pull`
- Shaping: `none`
- Streams: `1`
- Compression: `adaptive zstd`

## Corpus

- Schema: `xsync.manifest.v1`
- Manifest: `5588a3f1e7d770113191ba1e92296e96817ce8ba27364cf088baacb9c36e602a`
- Description: real corpus=large1gb pinned_digest=5588a3f1e7d770113191ba1e92296e96817ce8ba27364cf088baacb9c36e602a

## Tools

- **xsync** `xs 0.1.0 (943b8ec49bc2-dirty 2026-08-31) aarch64-apple-darwin`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 8.562747s | 0.083448s | 4.143984s | 259211264 | 5 | 887980892 | 103702807.634 B/s | 0 | - | 3 | pass |
| xsync | 8.414532s | 0.061544s | 3.067952s | 149618688 | 5 | 887980892 | 105529438.742 B/s | 887995366 | 1.016x | 3 | pass |

## Repetitions

| method | rep | order | cache | wall | durable | CPU | endpoint CPU | RSS | endpoint RSS | cache resident/total | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 8.646195s | 8.646704s | 4.040312s | 0.000000s | 236371968 | 0 | 887980892/887980892 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 1 | 1 | warm | 7.985581s | 7.985907s | 4.262597s | 0.000000s | 217268224 | 0 | 887980892/887980892 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 2 | 0 | warm | 8.562747s | 8.563163s | 4.143984s | 0.000000s | 259211264 | 0 | 887980892/887980892 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| xsync | 0 | 1 | first_pass | 8.509565s | 8.509974s | 2.962714s | 0.000000s | 102301696 | 0 | 887980892/887980892 | 5 | 887980892 | 887980892 | 887980892 | 887995366 | pass |
| xsync | 1 | 0 | warm | 8.352988s | 8.353422s | 3.067952s | 0.000000s | 117506048 | 0 | 887980892/887980892 | 5 | 887980892 | 887980892 | 887980892 | 887995366 | pass |
| xsync | 2 | 1 | warm | 8.414532s | 8.415139s | 3.459119s | 0.000000s | 149618688 | 0 | 887980892/887980892 | 5 | 887980892 | 887980892 | 887980892 | 887995366 | pass |
