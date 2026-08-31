# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1788212874215733000`
- Source revision: `943b8ec49bc2f3aa0f9a503598b86d3d68f24f57-dirty`
- Build: `4b89da9122c4b5a19f385a6e63d098bf` (`release`)

## Environment

- Hardware: `Apple M1 Max, 10 logical cores, 64 GiB RAM`
- OS: `macOS-26.6.2-arm64-arm-64bit-Mach-O`
- Kernel: `25.6.0`
- Filesystem: `tmpfs`
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
| rsync-a | 7.918576s | 0.025559s | 3.062843s | 25001984 | 5 | 887980892 | 112138966.569 B/s | 0 | - | 3 | pass |
| xsync | 8.291551s | 0.066923s | 2.812177s | 135397376 | 5 | 887980892 | 107094670.620 B/s | 887997013 | 0.958x | 3 | pass |

## Repetitions

| method | rep | order | cache | wall | durable | CPU | endpoint CPU | RSS | endpoint RSS | cache resident/total | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 7.881022s | 8.023057s | 2.978741s | 0.000000s | 20201472 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 1 | 1 | warm | 7.918576s | 8.062112s | 3.062843s | 0.000000s | 25001984 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| rsync-a | 2 | 0 | warm | 7.944135s | 8.076722s | 3.127360s | 0.000000s | 18726912 | 0 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 0 | pass |
| xsync | 0 | 1 | first_pass | 8.224628s | 8.371958s | 2.812177s | 2.568757s | 65929216 | 29556736 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
| xsync | 1 | 0 | warm | 8.455125s | 8.591617s | 2.867089s | 2.634952s | 55066624 | 29310976 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
| xsync | 2 | 1 | warm | 8.291551s | 8.430379s | 2.652110s | 2.617710s | 135397376 | 29540352 | 0/0 | 5 | 887980892 | 887980892 | 887980892 | 887997013 | pass |
