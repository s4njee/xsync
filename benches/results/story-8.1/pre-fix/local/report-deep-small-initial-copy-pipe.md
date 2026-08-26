# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787565633017347000`
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
- Manifest: `c4bbb1d39a822958eef248d6a45f442e3bb35ff00353094e853e2c88a33c6c8a`
- Description: class=deep-small tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.264281s | 0.017785s | 0.232690s | 5242880 | 1001 | 61380 | 0 | - | 5 | pass |
| rsync-az | 0.272017s | 0.000974s | 0.234969s | 6422528 | 1001 | 61380 | 0 | 0.958x | 5 | pass |
| xsync | 0.367348s | 0.013776s | 0.342286s | 6520832 | 1001 | 61380 | 93928 | 0.697x | 5 | pass |
| xsync-raw | 0.366192s | 0.009803s | 0.348595s | 6225920 | 1001 | 61380 | 112860 | 0.707x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.287534s | 0.232212s | 5193728 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.264281s | 0.232690s | 5242880 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.246496s | 0.235693s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.266955s | 0.235751s | 5242880 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.244100s | 0.232045s | 5144576 | 1001 | 61380 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.272017s | 0.238215s | 6422528 | 1001 | 61380 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.276000s | 0.237988s | 6340608 | 1001 | 61380 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.272347s | 0.232365s | 6373376 | 1001 | 61380 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.271043s | 0.234969s | 6389760 | 1001 | 61380 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.267290s | 0.232470s | 6324224 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.367348s | 0.342286s | 6340608 | 1001 | 61380 | 93928 | pass |
| xsync | 1 | 1 | warm | 0.348129s | 0.335646s | 6520832 | 1001 | 61380 | 93928 | pass |
| xsync | 2 | 0 | warm | 0.353572s | 0.341877s | 6307840 | 1001 | 61380 | 93928 | pass |
| xsync | 3 | 3 | warm | 0.387020s | 0.366436s | 6488064 | 1001 | 61380 | 93928 | pass |
| xsync | 4 | 2 | warm | 0.375572s | 0.357357s | 6356992 | 1001 | 61380 | 93928 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.374021s | 0.355957s | 6209536 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 1 | 2 | warm | 0.356389s | 0.337719s | 6225920 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 2 | 1 | warm | 0.366192s | 0.348595s | 6225920 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 3 | 0 | warm | 0.419796s | 0.355647s | 6193152 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 4 | 3 | warm | 0.345294s | 0.330402s | 6209536 | 1001 | 61380 | 112860 | pass |
