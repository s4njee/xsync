# xsync remote framing spike

- Schema: `xsync.remote-bench.report.v1`
- Host/filesystem/profile: `sanjee@mars.local` / `ext4` / `native-compressible`
- Corpus: `cdb0aa7bb26638d0d957a165317be65cc4e1f934bd4f75a86de129a139daa52c` (268435456 bytes)
- Wall time includes SSH setup, transfer/teardown, and an independent remote manifest.

| Method | Transport | Streams | Compression | Median wall | MAD | Setup | Wire | Paired speedup |
|---|---|---:|---|---:|---:|---:|---:|---:|
| rsync-a | reference-rsync-client | 1 | none | 4.324753s | 0.149950s | 0.158217s | 268514411 | - |
| rsync-az | reference-rsync-client | 1 | rsync -z negotiated default | 0.650314s | 0.066267s | 0.156181s | 42913 | 6.650x |
| xsync-1 | native-xsync-framing-spike | 1 | none | 4.286269s | 0.115423s | 0.150931s | 268482672 | 0.969x |
| xsync-adaptive-1 | native-xsync-framing-spike | 1 | adaptive-zstd-3 sample=65536 | 0.982064s | 0.011058s | 0.146811s | 83056 | 0.601x |

## Transport capability matrix

| Backend | Status | Dialect | Features | Correctness | Degraded guarantees |
|---|---|---|---|---|---|
| native_xsync | benchmark_spike_only | xsync-story-0.5-spike-v1 | bounded frames, 1/2/4/8 persistent data sessions, BLAKE3 before atomic publication, adaptive zstd | independent manifest required for every sample | flat regular-file corpus only, no durable resume, not production protocol v1 |
| rsync_protocol_fallback | not_implemented_story_4.5 | reference receiver protocol 32 | reference rsync -a/-az measured, server setup marker measured | reference client samples independently manifested; native codec unavailable | no xsync BLAKE3 framing, no xsync checkpoint resume, single stream, native fallback setup time unavailable until Story 4.5 |

## Repetitions

| Method | Rep | Order | Wall | CPU | RSS | Wire | Setup | Verify | Oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | 4.133984s | 1.035752s | 106463232 | 268514411 | 0.158217s | 0.209790s | pass |
| rsync-a | 1 | 0 | 4.434907s | 1.075855s | 134234112 | 268514411 | 0.160675s | 0.205920s | pass |
| rsync-a | 2 | 2 | 4.174803s | 1.064597s | 45170688 | 268514411 | 0.153323s | 0.221219s | pass |
| rsync-a | 3 | 2 | 4.886036s | 1.043072s | 88621056 | 268514411 | 0.149463s | 0.358205s | pass |
| rsync-a | 4 | 0 | 4.324753s | 1.050222s | 86327296 | 268514411 | 0.161415s | 0.246409s | pass |
| rsync-az | 0 | 1 | 1.116978s | 0.123077s | 8978432 | 42913 | 0.152657s | 0.714707s | pass |
| rsync-az | 1 | 3 | 2.107323s | 0.119063s | 8978432 | 42913 | 0.967658s | 0.892914s | pass |
| rsync-az | 2 | 3 | 0.584047s | 0.112625s | 8962048 | 42913 | 0.145892s | 0.202199s | pass |
| rsync-az | 3 | 1 | 0.588929s | 0.111927s | 8962048 | 42913 | 0.156181s | 0.197651s | pass |
| rsync-az | 4 | 1 | 0.650314s | 0.108614s | 8978432 | 42913 | 0.167548s | 0.213600s | pass |
| xsync-1 | 0 | 2 | 4.278437s | 0.890051s | 89276416 | 268482672 | 0.155999s | 0.203386s | pass |
| xsync-1 | 1 | 2 | 4.577409s | 0.897321s | 89243648 | 268482672 | 0.138592s | 0.368394s | pass |
| xsync-1 | 2 | 0 | 4.170845s | 0.875695s | 77742080 | 268482672 | 0.150931s | 0.244249s | pass |
| xsync-1 | 3 | 0 | 4.286269s | 0.866410s | 77479936 | 268482672 | 0.139860s | 0.203742s | pass |
| xsync-1 | 4 | 2 | 6.322919s | 0.903749s | 62603264 | 268482672 | 0.180089s | 0.294629s | pass |
| xsync-adaptive-1 | 0 | 3 | 0.982064s | 0.426605s | 9797632 | 83056 | 0.146185s | 0.202547s | pass |
| xsync-adaptive-1 | 1 | 1 | 0.981072s | 0.433986s | 9715712 | 83056 | 0.146811s | 0.197881s | pass |
| xsync-adaptive-1 | 2 | 1 | 0.971006s | 0.426561s | 9781248 | 83056 | 0.143345s | 0.204391s | pass |
| xsync-adaptive-1 | 3 | 3 | 1.493281s | 0.424528s | 9699328 | 83056 | 0.176625s | 0.714151s | pass |
| xsync-adaptive-1 | 4 | 3 | 1.662248s | 0.434810s | 10944512 | 83056 | 0.760895s | 0.236600s | pass |
