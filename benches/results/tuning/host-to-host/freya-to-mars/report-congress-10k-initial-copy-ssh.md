# xsync Benchmark Report

- Schema: `xsync.bench.report.v1`
- Generated (Unix ns): `1787923178448266749`
- Source revision: `unknown`
- Build: `ad64a481e5bcaa77401dd253d0dd9ab3` (`release`)

## Environment

- Hardware: `x86_64, 24 logical cores`
- OS: `Linux-7.1.6-arch1-1-x86_64-with-glibc2.44`
- Kernel: `7.1.6-arch1-1`
- Filesystem: `ext4 NVMe destination`
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
| rsync-a | 4.102852s | 0.113287s | 1.008001s | 105472000 | 22568 | 96542108 | 23530485.835 B/s | 0 | - | 5 | pass (includes sampled) |
| xsync | 4.794427s | 0.221668s | 1.022885s | 105472000 | 22568 | 96542108 | 20136317.473 B/s | 22915621 | 0.840x | 5 | pass (includes sampled) |

## Repetitions

| method | rep | order | cache | wall | CPU | RSS | items | logical | source allocated | destination allocated | wire | oracle |
|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | first_pass | 3.989565s | 1.002341s | 98357248 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass |
| rsync-a | 1 | 1 | warm | 4.102852s | 1.008001s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass (sampled) |
| rsync-a | 2 | 0 | warm | 3.984433s | 1.011468s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass (sampled) |
| rsync-a | 3 | 1 | warm | 5.786000s | 1.019608s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass (sampled) |
| rsync-a | 4 | 0 | warm | 4.154170s | 1.007416s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 0 | pass (sampled) |
| xsync | 0 | 1 | first_pass | 4.747828s | 1.045700s | 98357248 | 22568 | 96542108 | 96542108 | 96542108 | 22915621 | pass |
| xsync | 1 | 0 | warm | 5.064757s | 1.020654s | 102035456 | 22568 | 96542108 | 96542108 | 96542108 | 22915621 | pass (sampled) |
| xsync | 2 | 1 | warm | 4.794427s | 1.022885s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 22915621 | pass (sampled) |
| xsync | 3 | 0 | warm | 5.166821s | 1.056615s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 22915621 | pass (sampled) |
| xsync | 4 | 1 | warm | 4.572759s | 1.013165s | 105472000 | 22568 | 96542108 | 96542108 | 96542108 | 22915621 | pass (sampled) |
