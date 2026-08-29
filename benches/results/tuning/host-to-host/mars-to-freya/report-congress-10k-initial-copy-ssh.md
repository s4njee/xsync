# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787923061877152769`
- Source revision: `unknown`
- Build: `ad64a481e5bcaa77401dd253d0dd9ab3` (`release`)

## Environment

- Hardware: `x86_64, 24 logical cores`
- OS: `Linux-7.1.6-arch1-1-x86_64-with-glibc2.44`
- Kernel: `7.1.6-arch1-1`
- Filesystem: `ZFS zpcachyos lz4`
- Transport: `ssh to sanjee@192.168.1.156`
- Route: `ssh`
- Shaping: `none`
- Streams: `1`
- Compression: `adaptive zstd`

## Corpus

- Schema: `xsync.manifest.v1`
- Manifest: `fa7f75f7ef1ce81cb06af7492e51e58d5ae665769d9d5e92e33949f775b95e2e`
- Description: real corpus=congress-10k pinned_digest=fa7f75f7ef1ce81cb06af7492e51e58d5ae665769d9d5e92e33949f775b95e2e

## Tools

- **xsync** `xsync 0.1.0`: `xsync --progress-json SRC/ DEST/`
- **rsync** `rsync  version 3.4.4  protocol version 32`: `rsync -a SRC/ DEST/`

## Results

| method | median wall | MAD | CPU | peak RSS | items | logical bytes | allocated throughput | wire bytes | paired speedup | reps | oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 19.270489s | 0.282715s | 0.515447s | 103739392 | 22568 | 96542108 | 5009842.038 B/s | 0 | - | 5 | pass |
| xsync | 7.735250s | 0.123497s | 0.985126s | 103739392 | 22568 | 96542108 | 12480799.229 B/s | 22972845 | 2.471x | 5 | pass |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 19.426556s | 0.499579s | 98594816 | 22568 | 96542108 | 96542108 | 0 | 0 | pass |
| rsync-a | 1 | 1 | warm | 19.270489s | 0.515447s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 0 | pass |
| rsync-a | 2 | 0 | warm | 17.879137s | 0.510168s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 0 | pass |
| rsync-a | 3 | 1 | warm | 17.307645s | 0.517622s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 0 | pass |
| rsync-a | 4 | 0 | warm | 19.553205s | 0.517109s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 0 | pass |
| xsync | 0 | 1 | first_pass | 7.812033s | 0.949306s | 98623488 | 22568 | 96542108 | 96542108 | 0 | 22972845 | pass |
| xsync | 1 | 0 | warm | 7.611754s | 0.985126s | 101888000 | 22568 | 96542108 | 96542108 | 0 | 22972845 | pass |
| xsync | 2 | 1 | warm | 7.735250s | 1.012670s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 22972845 | pass |
| xsync | 3 | 0 | warm | 7.514025s | 0.984132s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 22972845 | pass |
| xsync | 4 | 1 | warm | 7.914283s | 0.996632s | 103739392 | 22568 | 96542108 | 96542108 | 0 | 22972845 | pass |
