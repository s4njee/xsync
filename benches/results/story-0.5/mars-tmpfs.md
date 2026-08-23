# xsync remote framing spike

- Schema: `xsync.remote-bench.report.v1`
- Host/filesystem/profile: `sanjee@mars.local` / `tmpfs` / `native`
- Corpus: `c211a30a72e305e2ba51b327282680459782a0aed26f7d5b1e0fb36b49083cd6` (268435456 bytes)
- Wall time includes SSH setup, transfer/teardown, and an independent remote manifest.

| Method | Transport | Streams | Compression | Median wall | MAD | Setup | Wire | Paired speedup |
|---|---|---:|---|---:|---:|---:|---:|---:|
| rsync-a | reference-rsync-client | 1 | none | 4.474045s | 0.332584s | 0.158577s | 268514411 | - |
| rsync-az | reference-rsync-client | 1 | rsync -z negotiated default | 4.218866s | 0.158860s | 0.170436s | 268512969 | 1.017x |
| xsync-1 | native-xsync-framing-spike | 1 | none | 4.388649s | 0.210879s | 0.146666s | 268482672 | 1.000x |
| xsync-2 | native-xsync-framing-spike | 2 | none | 4.018674s | 0.288076s | 0.166460s | 268482784 | 1.093x |
| xsync-4 | native-xsync-framing-spike | 4 | none | 3.946832s | 0.241645s | 0.186420s | 268483008 | 1.116x |
| xsync-8 | native-xsync-framing-spike | 8 | none | 4.529109s | 0.150528s | 0.282167s | 268483456 | 0.938x |
| xsync-adaptive-1 | native-xsync-framing-spike | 1 | adaptive-zstd-3 sample=65536 | 4.164531s | 0.118361s | 0.158063s | 268482672 | 0.963x |

## Transport capability matrix

| Backend | Status | Dialect | Features | Correctness | Degraded guarantees |
|---|---|---|---|---|---|
| native_xsync | benchmark_spike_only | xsync-story-0.5-spike-v1 | bounded frames, 1/2/4/8 persistent data sessions, BLAKE3 before atomic publication, adaptive zstd | independent manifest required for every sample | flat regular-file corpus only, no durable resume, not production protocol v1 |
| rsync_protocol_fallback | not_implemented_story_4.5 | reference receiver protocol 32 | reference rsync -a/-az measured, server setup marker measured | reference client samples independently manifested; native codec unavailable | no xsync BLAKE3 framing, no xsync checkpoint resume, single stream, native fallback setup time unavailable until Story 4.5 |

## Repetitions

| Method | Rep | Order | Wall | CPU | RSS | Wire | Setup | Verify | Oracle |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rsync-a | 0 | 0 | 4.474045s | 1.149221s | 110657536 | 268514411 | 0.172006s | 0.242691s | pass |
| rsync-a | 1 | 0 | 4.178352s | 1.148130s | 111722496 | 268514411 | 0.158577s | 0.242856s | pass |
| rsync-a | 2 | 5 | 5.194266s | 1.217133s | 109264896 | 268514411 | 0.173466s | 0.222750s | pass |
| rsync-a | 3 | 2 | 4.806628s | 1.036274s | 76808192 | 268514411 | 0.148858s | 0.235522s | pass |
| rsync-a | 4 | 3 | 3.911291s | 1.051869s | 86933504 | 268514411 | 0.151971s | 0.210293s | pass |
| rsync-az | 0 | 1 | 4.740833s | 1.392375s | 93110272 | 268512969 | 0.162954s | 0.233825s | pass |
| rsync-az | 1 | 6 | 4.107384s | 1.406252s | 47497216 | 268512969 | 0.170436s | 0.239895s | pass |
| rsync-az | 2 | 6 | 4.218866s | 1.350601s | 112885760 | 268512969 | 0.215810s | 0.230967s | pass |
| rsync-az | 3 | 1 | 4.377726s | 1.158065s | 123863040 | 268512969 | 0.178794s | 0.218527s | pass |
| rsync-az | 4 | 4 | 3.895904s | 1.144230s | 74940416 | 268512969 | 0.164378s | 0.233480s | pass |
| xsync-1 | 0 | 2 | 4.404717s | 1.033177s | 72253440 | 268482672 | 0.140835s | 0.264766s | pass |
| xsync-1 | 1 | 5 | 4.177770s | 1.214604s | 74727424 | 268482672 | 0.146666s | 0.222520s | pass |
| xsync-1 | 2 | 0 | 4.388649s | 1.091431s | 83525632 | 268482672 | 0.164574s | 0.269754s | pass |
| xsync-1 | 3 | 0 | 5.065445s | 0.868560s | 74285056 | 268482672 | 0.143889s | 0.239081s | pass |
| xsync-1 | 4 | 5 | 4.044519s | 0.825384s | 75890688 | 268482672 | 0.148398s | 0.222284s | pass |
| xsync-2 | 0 | 3 | 4.031282s | 1.154809s | 83034112 | 268482784 | 0.176925s | 0.241755s | pass |
| xsync-2 | 1 | 4 | 3.730598s | 1.102944s | 93962240 | 268482784 | 0.166460s | 0.241914s | pass |
| xsync-2 | 2 | 1 | 4.311337s | 1.112067s | 90488832 | 268482784 | 0.158085s | 0.265759s | pass |
| xsync-2 | 3 | 6 | 4.018674s | 0.898220s | 84230144 | 268482784 | 0.180068s | 0.214598s | pass |
| xsync-2 | 4 | 6 | 3.702681s | 0.907505s | 78036992 | 268482784 | 0.158444s | 0.210397s | pass |
| xsync-4 | 0 | 4 | 3.946832s | 1.187436s | 88358912 | 268483008 | 0.218336s | 0.241407s | pass |
| xsync-4 | 1 | 3 | 3.644298s | 1.193263s | 89849856 | 268483008 | 0.180038s | 0.222532s | pass |
| xsync-4 | 2 | 2 | 4.485634s | 1.220648s | 98238464 | 268483008 | 0.183707s | 0.215802s | pass |
| xsync-4 | 3 | 5 | 4.013873s | 0.988796s | 102645760 | 268483008 | 0.201661s | 0.227656s | pass |
| xsync-4 | 4 | 0 | 3.705187s | 0.975144s | 86212608 | 268483008 | 0.186420s | 0.217187s | pass |
| xsync-8 | 0 | 5 | 4.146392s | 1.436376s | 137445376 | 268483456 | 0.260871s | 0.240904s | pass |
| xsync-8 | 1 | 2 | 4.529109s | 1.389778s | 99401728 | 268483456 | 0.283342s | 0.725001s | pass |
| xsync-8 | 2 | 3 | 4.679637s | 1.440155s | 144277504 | 268483456 | 0.208102s | 0.239114s | pass |
| xsync-8 | 3 | 4 | 4.308828s | 1.273998s | 91684864 | 268483456 | 0.282167s | 0.215901s | pass |
| xsync-8 | 4 | 1 | 4.535135s | 1.215205s | 88768512 | 268483456 | 0.286336s | 0.722335s | pass |
| xsync-adaptive-1 | 0 | 6 | 4.071019s | 1.139571s | 69926912 | 268482672 | 0.158063s | 0.260592s | pass |
| xsync-adaptive-1 | 1 | 1 | 4.164531s | 1.102681s | 88244224 | 268482672 | 0.145493s | 0.229693s | pass |
| xsync-adaptive-1 | 2 | 4 | 4.652322s | 1.060647s | 81428480 | 268482672 | 0.160503s | 0.218600s | pass |
| xsync-adaptive-1 | 3 | 3 | 5.666350s | 0.899940s | 83558400 | 268482672 | 0.144030s | 0.222753s | pass |
| xsync-adaptive-1 | 4 | 2 | 4.046170s | 0.864285s | 77725696 | 268482672 | 0.164019s | 0.218958s | pass |
