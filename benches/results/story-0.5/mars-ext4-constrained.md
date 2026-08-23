# xsync remote framing spike

- Schema: `xsync.remote-bench.report.v1`
- Host/filesystem/profile: `sanjee@mars.local` / `ext4` / `constrained-receiver-1cpu-512mib`
- Corpus: `c211a30a72e305e2ba51b327282680459782a0aed26f7d5b1e0fb36b49083cd6` (268435456 bytes)
- Wall time includes SSH setup, transfer/teardown, and an independent remote manifest.

| Method | Transport | Streams | Compression | Median wall | MAD | Setup | Wire | Paired speedup |
|---|---|---:|---|---:|---:|---:|---:|---:|
| rsync-a | reference-rsync-client | 1 | none | 4.392767s | 0.236076s | 0.166839s | 268514411 | - |
| xsync-1 | native-xsync-framing-spike | 1 | none | 4.222387s | 0.086790s | 0.186932s | 268482672 | 1.026x |
| xsync-4 | native-xsync-framing-spike | 4 | none | 4.510556s | 0.446925s | 0.705389s | 268483008 | 1.025x |

## Transport capability matrix

| Backend | Status | Dialect | Features | Correctness | Degraded guarantees |
|---|---|---|---|---|---|
| native_xsync | benchmark_spike_only | xsync-story-0.5-spike-v1 | bounded frames, 1/2/4/8 persistent data sessions, BLAKE3 before atomic publication, adaptive zstd | independent manifest required for every sample | flat regular-file corpus only, no durable resume, not production protocol v1 |
| rsync_protocol_fallback | not_implemented_story_4.5 | reference receiver protocol 32 | reference rsync -a/-az measured, server setup marker measured | reference client samples independently manifested; native codec unavailable | no xsync BLAKE3 framing, no xsync checkpoint resume, single stream, native fallback setup time unavailable until Story 4.5 |

## Repetitions

| Method | Rep | Order | Wall | CPU | RSS | Wire | Setup | Verify | Oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | 4.392767s | 0.994924s | 89948160 | 268514411 | 0.165976s | 0.355654s | pass |
| rsync-a | 1 | 0 | 5.189959s | 0.989575s | 98254848 | 268514411 | 1.022706s | 0.208453s | pass |
| rsync-a | 2 | 1 | 4.935275s | 0.970017s | 127696896 | 268514411 | 0.796237s | 0.209610s | pass |
| rsync-a | 3 | 2 | 4.333549s | 1.007725s | 156745728 | 268514411 | 0.166839s | 0.220284s | pass |
| rsync-a | 4 | 2 | 4.156691s | 0.985899s | 100220928 | 268514411 | 0.160110s | 0.202969s | pass |
| xsync-1 | 0 | 1 | 4.135597s | 0.851638s | 75939840 | 268482672 | 0.151950s | 0.222408s | pass |
| xsync-1 | 1 | 2 | 4.727302s | 0.842101s | 76398592 | 268482672 | 0.317538s | 0.415304s | pass |
| xsync-1 | 2 | 2 | 5.491395s | 0.851135s | 96665600 | 268482672 | 0.957834s | 0.662758s | pass |
| xsync-1 | 3 | 1 | 4.222387s | 0.831081s | 88752128 | 268482672 | 0.186932s | 0.211491s | pass |
| xsync-1 | 4 | 0 | 4.166990s | 0.831681s | 88735744 | 268482672 | 0.152326s | 0.208542s | pass |
| xsync-4 | 0 | 2 | 4.034432s | 1.045817s | 94814208 | 268483008 | 0.194520s | 0.200943s | pass |
| xsync-4 | 1 | 1 | 4.957481s | 0.993546s | 88473600 | 268483008 | 0.705389s | 0.705941s | pass |
| xsync-4 | 2 | 0 | 4.554879s | 1.001070s | 100515840 | 268483008 | 0.790884s | 0.204568s | pass |
| xsync-4 | 3 | 0 | 4.510556s | 1.022334s | 94846976 | 268483008 | 0.797987s | 0.212654s | pass |
| xsync-4 | 4 | 1 | 4.037409s | 0.999496s | 82853888 | 268483008 | 0.207672s | 0.225872s | pass |
