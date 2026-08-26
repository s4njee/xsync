# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787568219844146000`
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
- Manifest: `c4bbb1d39a822958eef248d6a45f442e3bb35ff00353094e853e2c88a33c6c8a`
- Description: class=deep-small tier=smoke workload=initial-copy seed=42

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0.269063s | 0.001047s | 0.233908s | 5210112 | 1001 | 61380 | 0 | - | 5 | pass |
| rsync-az | 0.259644s | 0.007214s | 0.230051s | 6406144 | 1001 | 61380 | 0 | 1.011x | 5 | pass |
| xsync | 0.296109s | 0.003103s | 0.286245s | 7061504 | 1001 | 61380 | 93060 | 0.888x | 5 | pass |
| xsync-raw | 0.307950s | 0.013605s | 0.293032s | 7028736 | 1001 | 61380 | 112860 | 0.869x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 0.262910s | 0.216847s | 5177344 | 1001 | 61380 | 0 | pass |
| rsync-a | 1 | 3 | warm | 0.270110s | 0.233908s | 5193728 | 1001 | 61380 | 0 | pass |
| rsync-a | 2 | 2 | warm | 0.269063s | 0.234664s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 3 | 1 | warm | 0.255163s | 0.226132s | 5210112 | 1001 | 61380 | 0 | pass |
| rsync-a | 4 | 0 | warm | 0.269341s | 0.234750s | 5128192 | 1001 | 61380 | 0 | pass |
| rsync-az | 0 | 1 | first_pass | 0.274018s | 0.242839s | 6340608 | 1001 | 61380 | 0 | pass |
| rsync-az | 1 | 0 | warm | 0.258983s | 0.223687s | 6406144 | 1001 | 61380 | 0 | pass |
| rsync-az | 2 | 3 | warm | 0.271475s | 0.237297s | 6373376 | 1001 | 61380 | 0 | pass |
| rsync-az | 3 | 2 | warm | 0.252430s | 0.220895s | 6340608 | 1001 | 61380 | 0 | pass |
| rsync-az | 4 | 1 | warm | 0.259644s | 0.230051s | 6324224 | 1001 | 61380 | 0 | pass |
| xsync | 0 | 2 | first_pass | 0.296109s | 0.283554s | 7061504 | 1001 | 61380 | 93060 | pass |
| xsync | 1 | 1 | warm | 0.290206s | 0.281869s | 6848512 | 1001 | 61380 | 93060 | pass |
| xsync | 2 | 0 | warm | 0.299211s | 0.294115s | 6930432 | 1001 | 61380 | 93060 | pass |
| xsync | 3 | 3 | warm | 0.294892s | 0.286245s | 6750208 | 1001 | 61380 | 93060 | pass |
| xsync | 4 | 2 | warm | 0.318858s | 0.310135s | 6619136 | 1001 | 61380 | 93060 | pass |
| xsync-raw | 0 | 3 | first_pass | 0.283469s | 0.276387s | 6979584 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 1 | 2 | warm | 0.294345s | 0.286738s | 6701056 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 2 | 1 | warm | 0.348670s | 0.308548s | 6520832 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 3 | 0 | warm | 0.307950s | 0.293032s | 7028736 | 1001 | 61380 | 112860 | pass |
| xsync-raw | 4 | 3 | warm | 0.309782s | 0.297215s | 7028736 | 1001 | 61380 | 112860 | pass |
