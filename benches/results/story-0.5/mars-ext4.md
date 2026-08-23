# xsync remote framing spike

- Schema: `xsync.remote-bench.report.v1`
- Host/filesystem/profile: `sanjee@mars.local` / `ext4` / `native`
- Corpus: `c211a30a72e305e2ba51b327282680459782a0aed26f7d5b1e0fb36b49083cd6` (268435456 bytes)
- Wall time includes SSH setup, transfer/teardown, and an independent remote manifest.

| Method | Transport | Streams | Compression | Median wall | MAD | Setup | Wire | Paired speedup |
|---|---|---:|---|---:|---:|---:|---:|---:|
| rsync-a | reference-rsync-client | 1 | none | 5.135457s | 0.215568s | 0.313066s | 268514411 | - |
| rsync-az | reference-rsync-client | 1 | rsync -z negotiated default | 4.348315s | 0.225699s | 0.164247s | 268512969 | 1.036x |
| xsync-1 | native-xsync-framing-spike | 1 | none | 4.602318s | 0.088639s | 0.581450s | 268482672 | 1.106x |
| xsync-2 | native-xsync-framing-spike | 2 | none | 4.779186s | 0.734061s | 0.666176s | 268482784 | 0.983x |
| xsync-4 | native-xsync-framing-spike | 4 | none | 5.128293s | 0.728291s | 0.218146s | 268483008 | 1.026x |
| xsync-8 | native-xsync-framing-spike | 8 | none | 5.116965s | 0.859034s | 0.270503s | 268483456 | 1.036x |
| xsync-adaptive-1 | native-xsync-framing-spike | 1 | adaptive-zstd-3 sample=65536 | 5.074564s | 0.698345s | 0.158800s | 268482672 | 1.056x |

## Transport capability matrix

| Backend | Status | Dialect | Features | Correctness | Degraded guarantees |
|---|---|---|---|---|---|
| native_xsync | benchmark_spike_only | xsync-story-0.5-spike-v1 | bounded frames, 1/2/4/8 persistent data sessions, BLAKE3 before atomic publication, adaptive zstd | independent manifest required for every sample | flat regular-file corpus only, no durable resume, not production protocol v1 |
| rsync_protocol_fallback | not_implemented_story_4.5 | reference receiver protocol 32 | reference rsync -a/-az measured, server setup marker measured | reference client samples independently manifested; native codec unavailable | no xsync BLAKE3 framing, no xsync checkpoint resume, single stream, native fallback setup time unavailable until Story 4.5 |

## Repetitions

| Method | Rep | Order | Wall | CPU | RSS | Wire | Setup | Verify | Oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | 6.549834s | 1.457328s | 134627328 | 268514411 | 0.313066s | 0.217941s | pass |
| rsync-a | 1 | 0 | 5.135457s | 1.082791s | 93388800 | 268514411 | 0.159836s | 0.210297s | pass |
| rsync-a | 2 | 5 | 5.263753s | 1.248369s | 109887488 | 268514411 | 0.673613s | 0.648755s | pass |
| rsync-a | 3 | 2 | 4.919889s | 1.335546s | 105283584 | 268514411 | 1.063203s | 0.211645s | pass |
| rsync-a | 4 | 3 | 4.337489s | 1.269787s | 115736576 | 268514411 | 0.170817s | 0.211550s | pass |
| rsync-az | 0 | 1 | 6.800737s | 1.366937s | 92192768 | 268512969 | 0.506848s | 0.548057s | pass |
| rsync-az | 1 | 6 | 4.122617s | 1.148001s | 75071488 | 268512969 | 0.162200s | 0.217269s | pass |
| rsync-az | 2 | 6 | 5.284224s | 1.413209s | 97828864 | 268512969 | 0.827730s | 0.817286s | pass |
| rsync-az | 3 | 1 | 4.348315s | 1.394993s | 61374464 | 268512969 | 0.164247s | 0.713483s | pass |
| rsync-az | 4 | 4 | 4.185754s | 1.409947s | 112066560 | 268512969 | 0.157983s | 0.233567s | pass |
| xsync-1 | 0 | 2 | 8.022280s | 0.858096s | 81346560 | 268482672 | 0.581450s | 0.261130s | pass |
| xsync-1 | 1 | 5 | 4.642685s | 0.919931s | 83247104 | 268482672 | 0.151336s | 0.214879s | pass |
| xsync-1 | 2 | 0 | 4.602318s | 0.895939s | 69779456 | 268482672 | 0.672141s | 0.212923s | pass |
| xsync-1 | 3 | 0 | 3.974831s | 1.172338s | 71729152 | 268482672 | 0.157775s | 0.220757s | pass |
| xsync-1 | 4 | 5 | 4.513679s | 1.137195s | 88440832 | 268482672 | 0.656519s | 0.208915s | pass |
| xsync-2 | 0 | 3 | 7.150653s | 0.878417s | 73990144 | 268482784 | 0.696119s | 0.252342s | pass |
| xsync-2 | 1 | 4 | 4.779186s | 1.000618s | 94568448 | 268482784 | 0.675170s | 0.448682s | pass |
| xsync-2 | 2 | 1 | 3.705386s | 0.961438s | 86949888 | 268482784 | 0.166143s | 0.332440s | pass |
| xsync-2 | 3 | 6 | 4.045125s | 1.266744s | 83427328 | 268482784 | 0.158434s | 0.215975s | pass |
| xsync-2 | 4 | 6 | 4.935740s | 1.176152s | 88424448 | 268482784 | 0.666176s | 0.726002s | pass |
| xsync-4 | 0 | 4 | 6.389391s | 1.009858s | 64389120 | 268483008 | 0.218146s | 0.233559s | pass |
| xsync-4 | 1 | 3 | 5.478160s | 1.036807s | 99975168 | 268483008 | 0.822072s | 0.352553s | pass |
| xsync-4 | 2 | 2 | 3.696234s | 1.283654s | 88997888 | 268483008 | 0.184842s | 0.261502s | pass |
| xsync-4 | 3 | 5 | 5.128293s | 1.288131s | 84131840 | 268483008 | 0.708697s | 0.731300s | pass |
| xsync-4 | 4 | 0 | 4.400002s | 1.308358s | 89554944 | 268483008 | 0.198841s | 0.722337s | pass |
| xsync-8 | 0 | 5 | 6.347919s | 1.283191s | 147554304 | 268483456 | 0.270503s | 0.255004s | pass |
| xsync-8 | 1 | 2 | 5.116965s | 1.254689s | 149176320 | 268483456 | 0.294584s | 0.220457s | pass |
| xsync-8 | 2 | 3 | 4.257931s | 1.495930s | 124813312 | 268483456 | 0.244868s | 0.279149s | pass |
| xsync-8 | 3 | 4 | 3.836197s | 1.491657s | 107167744 | 268483456 | 0.217570s | 0.239004s | pass |
| xsync-8 | 4 | 1 | 5.290583s | 1.513928s | 120815616 | 268483456 | 1.115764s | 0.258510s | pass |
| xsync-adaptive-1 | 0 | 6 | 6.443127s | 0.955664s | 70516736 | 268482672 | 0.158800s | 0.220607s | pass |
| xsync-adaptive-1 | 1 | 1 | 5.235138s | 0.917884s | 81903616 | 268482672 | 0.787230s | 0.211043s | pass |
| xsync-adaptive-1 | 2 | 4 | 4.376219s | 1.116894s | 77266944 | 268482672 | 0.156784s | 0.226966s | pass |
| xsync-adaptive-1 | 3 | 3 | 3.946193s | 1.158317s | 88965120 | 268482672 | 0.158038s | 0.217816s | pass |
| xsync-adaptive-1 | 4 | 2 | 5.074564s | 1.208585s | 79446016 | 268482672 | 0.649950s | 0.711055s | pass |
