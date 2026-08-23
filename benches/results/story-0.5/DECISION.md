# Story 0.5 decision — remote baseline and defaults

Date: 2026-08-23

## Decision

- Default to **one remote data stream** when `--streams` is omitted. Four streams produced a
  verified 1.116x paired speedup on tmpfs, but its 1.026x ext4 result had 21.4% relative paired
  MAD and is therefore unverified under the Story 0 benchmark rule. The requirement needs verified
  improvement on two materially different filesystems. The constrained ext4 receiver did not
  regress by 10%, but it is not a second filesystem class. Explicit stream counts remain honored.
- Use **adaptive zstd level 3**, a **64 KiB sample**, and a **95% selection threshold** as the
  compression default for later transport integration. The 64 KiB, 256 KiB, and 1 MiB samples made
  identical decisions on all four corpus classes. Incompressible input selected no files and added
  exactly 0% application-wire bytes, below the 2% limit.
- Keep native xsync framing and rsync-protocol fallback as separate capability rows. This story
  measures reference rsync 3.4.4 protocol 32, but the native fallback codec remains Story 4.5 work.
  No production fallback or production remote transport is claimed here.

`DEFAULT_REMOTE_STREAMS`, the CLI help, and compression sampling constants encode these decisions.
Runtime remote transport remains scheduled in Stories 4.2–4.5.

## Scope boundary

The production `xsync` CLI currently parses and validates requests; it does not yet transfer over
SSH. Story 0.5 therefore uses `xsync-remote-spike`, a release-built benchmark executable in the
workspace, rather than presenting tar or another program as xsync. Its bounded framing uses
persistent SSH receiver sessions, 1 MiB data frames, per-file BLAKE3, sibling staging, and atomic
publication. It intentionally supports only flat regular-file corpora and has no durable resume.
Those limitations are recorded in every report.

The `rsync -a` and `rsync -az` rows invoke the installed reference client and receiver directly.
The remote wrapper emits a server-ready marker so setup time is separately visible. Wire counts are
application bytes reported by the protocols, not encrypted SSH packet captures. Tar was omitted:
an archive stream would add a reference with different semantics and is not used as an xsync proxy.

## Stream-count evidence

All rows below use the same 256 MiB, 256-file deterministic incompressible corpus, five rotated and
reversed repetitions, and an independent destination manifest after every transfer. Wall time
includes SSH setup, transfer/teardown, and verification. The two native filesystem rows use the
same AMD Ryzen 9 7900X host; `/home/sanjee` is ext4 and `/tmp` is tmpfs.

| Destination/profile | Method | Median wall | Wall MAD | Paired vs xsync-1 | Paired MAD | Peak RSS |
|---|---|---:|---:|---:|---:|---:|
| ext4, native | xsync-1 | 4.6023 s | 0.0886 s | baseline | — | 88,440,832 B |
| ext4, native | xsync-4 | 5.1283 s | 0.7283 s | 1.026x | 0.219x (21.4%, unverified) | 99,975,168 B |
| tmpfs, native | xsync-1 | 4.3886 s | 0.2109 s | baseline | — | 83,525,632 B |
| tmpfs, native | xsync-4 | 3.9468 s | 0.2416 s | 1.116x | 0.030x | 102,645,760 B |
| ext4, 1 CPU/512 MiB receiver | xsync-1 | 4.2224 s | 0.0868 s | baseline | — | 96,665,600 B |
| ext4, 1 CPU/512 MiB receiver | xsync-4 | 4.5106 s | 0.4469 s | 1.025x | 0.072x | 100,515,840 B |

The constrained profile runs each receiver-side stream in a user systemd scope with
`CPUQuota=100%` and `MemoryMax=512M`, plus `taskset -c 0`. It is a real constrained receiver
process on Mars, not a claim that Mars is physically a small host. The previously used Raspberry
Pi validation host was DNS-unreachable on the measurement date.

Two and eight streams remain available for explicit use. On tmpfs, two measured 1.093x and eight
0.938x versus one; on ext4, their paired results were 0.983x and 1.036x. None changes the default
rule, which specifically admits four only after the two-filesystem gate passes.

## Compression evidence

The deterministic probe covered short files, compressible, incompressible, and mixed corpora:

| Corpus | Files selected at 64 KiB | Adaptive/raw application-wire ratio |
|---|---:|---:|
| short compressible files | 32/32 | 0.00102x |
| compressible | 256/256 | 0.00015x |
| incompressible | 0/256 | 1.00000x |
| mixed regular files | 4,102 selected | 0.51458x |

The 256 KiB and 1 MiB sample sizes produced the same selections and wire counts, so 64 KiB is the
least-work choice. A real compressible ext4 transfer reduced native spike wire bytes from
268,482,672 to 83,056 and improved the same-repetition native wall ratio by a 4.295x median.
Reference `rsync -az` used 42,913 application bytes and a 0.6503 s median versus the adaptive
spike's 0.9821 s, so the evidence does not claim superiority over rsync compression.

## Baselines and fallback capability

| Backend | Status/dialect | Measured features | Correctness | Degraded guarantees |
|---|---|---|---|---|
| Native xsync | benchmark spike only; `xsync-story-0.5-spike-v1` | bounded frames, 1/2/4/8 persistent sessions, BLAKE3 before publication, adaptive zstd | independent manifest after every sample | flat regular files only; no durable resume; not production protocol v1 |
| rsync fallback | native codec not implemented; reference protocol 32 measured | `rsync -a`/`-az`, server-ready setup marker | reference transfers independently manifested | no xsync BLAKE3 framing or checkpoints; single stream; native fallback setup unavailable until Story 4.5 |

Reference `rsync -a` medians were 5.1355 s on ext4, 4.4740 s on tmpfs, and 4.3928 s in the
constrained profile. Its median setup phases were 0.3131 s, 0.1586 s, and 0.1668 s respectively.
All reference and native samples passed exact item-count, logical-byte, metadata, and content
manifests.

## Artifacts

- `mars-ext4.json` / `.md`: native ext4 stream and baseline matrix.
- `mars-tmpfs.json` / `.md`: native tmpfs stream and baseline matrix.
- `mars-ext4-constrained.json` / `.md`: explicit 1-CPU/512-MiB receiver profile.
- `mars-ext4-compressible.json` / `.md`: actual adaptive-compression transfer evidence.
- `compression-sampling.json` / `.md`: the 64 KiB/256 KiB/1 MiB four-corpus selection matrix.

The JSON reports retain all repetitions, method order, setup/transfer/verification phases, CPU,
peak RSS, application-wire bytes, build identity, hardware/OS/kernel identity, and oracle results.
This workspace did not contain VCS metadata during measurement, so `source_revision` is explicitly
`unknown-no-vcs-metadata`; the release build ID and framing dialect remain recorded rather than a
revision being invented.
