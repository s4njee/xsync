# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1788212815729768000`
- Source revision: `943b8ec49bc2f3aa0f9a503598b86d3d68f24f57-dirty`
- Build: `4b89da9122c4b5a19f385a6e63d098bf` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.2-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `ext4`
- Transport: `ssh to root@orion.local`
- Route: `mbp-to-orion-router1`
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
| rsync-a | 7.925116s | 0.030192s | 3.199325s | 25133056 | 5 | 887980892 | 112046419.079 B/s | 0 | - | 3 | pass |
| xsync | 10.631778s | 0.110994s | 2.899772s | 76218368 | 5 | 887980892 | 83521389.472 B/s | 887997013 | 0.753x | 3 | pass |

## Repetitions

| method | rep | order | cache | wall | durable | CPU | endpoint CPU | RSS | endpoint RSS | cache resident/total | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 7.925116s | 8.563501s | 3.068603s | 0.000000s | 25133056 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 1 | 1 | warm | 7.894924s | 8.679053s | 3.199325s | 0.000000s | 22265856 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 2 | 0 | warm | 14.542791s | 15.330610s | 4.644969s | 0.000000s | 18006016 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| xsync | 0 | 1 | first_pass | 10.520784s | 10.685434s | 3.045006s | 2.789484s | 56016896 | 29540352 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
| xsync | 1 | 0 | warm | 10.948935s | 11.084691s | 2.899772s | 2.774606s | 56508416 | 29310976 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
| xsync | 2 | 1 | warm | 10.631778s | 10.773612s | 2.827788s | 2.882164s | 76218368 | 29540352 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
