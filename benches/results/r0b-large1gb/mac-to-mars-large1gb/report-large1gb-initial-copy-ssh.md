# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1788212727899457000`
- Source revision: `943b8ec49bc2f3aa0f9a503598b86d3d68f24f57-dirty`
- Build: `4b89da9122c4b5a19f385a6e63d098bf` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.2-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `ext4`
- Transport: `ssh to sanjee@mars.local`
- Route: `mbp-to-mars-switch`
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
| rsync-a | 8.610208s | 0.404853s | 2.886232s | 25100288 | 5 | 887980892 | 103131171.795 B/s | 0 | - | 3 | pass |
| xsync | 8.504843s | 0.532731s | 2.938952s | 69058560 | 5 | 887980892 | 104408852.776 B/s | 887997013 | 0.996x | 3 | pass |

## Repetitions

| method | rep | order | cache | wall | durable | CPU | endpoint CPU | RSS | endpoint RSS | cache resident/total | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 7.936429s | 8.246297s | 2.886232s | 0.000000s | 25100288 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 1 | 1 | warm | 9.015061s | 9.333239s | 2.827712s | 0.000000s | 18022400 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 2 | 0 | warm | 8.610208s | 8.957369s | 3.338065s | 0.000000s | 22724608 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| xsync | 0 | 1 | first_pass | 7.972112s | 8.612692s | 2.938952s | 0.879917s | 69058560 | 40017920 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
| xsync | 1 | 0 | warm | 8.504843s | 9.237252s | 2.884746s | 0.897269s | 55099392 | 39911424 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
| xsync | 2 | 1 | warm | 9.071241s | 11.051313s | 3.340756s | 0.884562s | 62390272 | 39923712 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
