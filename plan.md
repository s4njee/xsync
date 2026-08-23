# xsync v1 — evidence-driven high-performance sync over SSH

## Product thesis

`xsync` is a Rust sync engine with rsync-familiar path semantics, a bounded parallel pipeline,
verified atomic writes, and workload-specific transfer strategies. The goal is to be materially
faster than rsync where modern hardware and a framed protocol create a real advantage without
claiming a universal multiplier.

The previous plan claimed that rsync leaves 5–10x performance on the table. The `f2` experiments
do not support that as a general claim. On a 10,000-file compressible corpus, a single archive
stream was about 1.04–1.49x faster than `rsync -a`; favorable multi-stream runs reached roughly
1.6–2.4x. The 20–80x result was against per-file SFTP-style transfer, not rsync. Those runs also
compared compressed tar with uncompressed rsync and often used one repetition, so they establish
direction rather than a product promise.

Accordingly:

- Performance claims ship only from the `xsync` regression harness, against fair rsync baselines.
- Correctness, bounded memory, path safety, and crash behavior are release gates, not tradeoffs for
  throughput.
- Single-stream framing is implemented and measured before multi-stream tuning.
- Defaults come from a cross-host benchmark gate, not CPU count or an architectural assumption.

## Evidence imported from `f2`

These findings are relevant enough to constrain v1:

1. **Batching is first-order for small files.** A continuous archive-shaped stream avoids per-file
   network round trips. This validates logical small-file batches, but not any particular batch or
   frame size.
2. **Parallel streams are host- and filesystem-dependent.** Two or four streams often helped;
   eight sometimes regressed badly. `--streams 1` must remain a complete path and the shipping
   default must pass a multi-host decision gate.
3. **Destination filesystem behavior can dominate CPU.** Benchmarks must fingerprint and vary the
   destination filesystem instead of treating host core count as the capacity signal.
4. **Compression sampling works.** A bounded sample selected zstd for XML and disabled compression
   for random media. `xsync` will validate sample size, per-frame overhead, and mixed corpora using
   its own protocol.
5. **Real directory trees are much slower to scan than flat synthetic trees.** Scanner benchmarks
   require both shapes; a 100k-flat-files test is not representative by itself.
6. **Filesystem-native cloning is the dominant local fast path.** APFS tree cloning beat an
   ordinary copy by 22x, and large-file cloning was effectively constant-time. Local reflink/clone
   support belongs in v1 behind capability detection and correctness tests.
7. **Durable chunk identity enables real resume.** Deterministic temp names provide safe restart,
   but they do not prevent retransmitting a partially completed large file. Resume requires stable
   file/chunk identities and receiver checkpoints.
8. **Protocol paths cannot assume Unicode on Unix.** Raw pathname bytes must survive a round trip;
   cross-platform encoding and path-safety rules must be settled before the wire format freezes.

The FSEvents, persistent Finder index, and same-volume move results are not v1 sync requirements.
They inform the v2 daemon: events are hints with rescan-on-drop, full rescans are expensive, and a
persistent index is only useful when its invalidation contract is correct.

## v1 scope and semantics

- **Directions:** local→local, local→remote, and remote→local. Remote→remote is deferred.
- **Transport:** SSH first starts a compatible `xsync --server`; no daemon or listening port is
  required. If the remote endpoint does not have xsync and it cannot or will not be installed,
  `RsyncTransport` may launch the remote rsync server mode and speak the negotiated rsync wire
  protocol directly.
- **Update model:** whole-file transfer in v1. Default equality is type + size + mtime; `--checksum`
  uses BLAKE3. FastCDC delta transfer remains v2.
- **Archive-like metadata:** preserve mtimes, Unix permission bits, empty directories, and symlinks
  as symlinks. Hardlinks, ownership, ACLs, xattrs, resource forks, and sparse layout are explicit v1
  limitations.
- **Integrity:** bytes are hashed while read and verified before publication. `--paranoid` performs
  a destination readback. Kernel reflink/clone has no incoming byte stream, so it verifies source
  stability and operation success by default; `--paranoid` supplies byte readback.
- **Publication:** files are staged beside their destination and atomically renamed. A failed or
  interrupted job never exposes a truncated final pathname.
- **Resume:** local restart safety and remote durable resume are distinct contracts. Durable resume
  may resend at most the uncheckpointed window after a crash and must reject changed source files.
- **Memory:** scan, plan, transfer, compression, framing, and resume-state handling all have explicit
  bounds independent of corpus size.

## Path and source correctness

Wire paths are an ordered sequence of relative components rather than a UTF-8 `String` contract.
Unix components preserve raw bytes. Windows components use a documented reversible encoding. The
receiver rejects empty components, absolute/rooted forms, `.`/`..`, NUL, platform prefixes, duplicate
destinations under its case/normalization rules, and any traversal through a pre-existing symlink.
All destination operations are descriptor-relative where the platform permits it.

The scanner records a source fingerprint. Before copying, the reader opens without following a
symlink and checks the opened object against the fingerprint. It checks the descriptor and pathname
again after the final read. A change causes one fresh scan/retry; a second change is a named partial
failure. A valid BLAKE3 digest must never bless a mixture of two source versions.

## Engine pipeline (`xsync-core`)

1. **Discovery** — source and destination scans run with bounded queues. A fresh/absent destination
   can begin transferring immediately. Otherwise the destination index must be ready before exact
   classification; source discovery may spool into a bounded per-run index while that happens.
2. **Bounded destination index** — use an in-memory map below a measured memory budget and spill to
   a per-run on-disk index above it. A persistent index is not trusted as filesystem truth in v1.
3. **Plan** — classify new, changed, unchanged, type-replaced, and extraneous entries. Deletes are
   held until every transfer and verification succeeds.
4. **Strategy selection** — thresholds are initial hypotheses, not protocol constants:
   - small files: logical batches currently target 32 MiB and 8,192 entries;
   - medium files: whole-file work items;
   - large files: disjoint logical chunks;
   - local same-filesystem targets: attempt directory clone/reflink, then file reflink, then copy.
5. **Dispatch** — local I/O workers and network stream count are separate controls. Small and medium
   work uses a shared bounded queue so one slow worker cannot head-of-line block unrelated workers.
   Large-file ranges retain stable stream ownership where required by the transport.
6. **Write and verify** — deterministic staging, per-payload BLAKE3 verification, stable metadata,
   atomic publication, optional readback, and delayed deepest-first directory metadata.
7. **Events** — typed events report discovered/planned/transferred/skipped/deleted/failed items,
   logical bytes, wire bytes, retransmitted bytes, resume savings, compression choice, and fast-path
   use. The terminal and JSONL renderer consume the same stream.

## Protocol and flow control

The wire protocol is versioned and language-neutral even though both v1 endpoints are Rust. It uses
a fixed header and length-prefixed typed messages; serialization library types are not the protocol
specification.

Initial safety limits, to be confirmed before protocol freeze:

- maximum complete payload: 16 MiB;
- maximum data segment: 8 MiB;
- maximum encoded path: 1 MiB;
- maximum unacknowledged data: 32 MiB per session by default;
- bounded decompressed length declared before allocation.

Logical 32 MiB batches and large-file chunks may span multiple bounded protocol frames. Receivers
validate the header length before allocating or reading the body, reject trailing bytes and unknown
types/flags, and cap collection counts before reserving memory. Compression has both compressed and
decompressed size limits.

Stable resume identity is derived from the job/session plus a source fingerprint and relative path;
each data segment is identified by file ID + offset + length. The receiver checkpoints verified
ranges and staging identity atomically at a reference interval of 64 MiB. Replayed acknowledged
segments are idempotent. Resume-state pages are bounded, and changed source fingerprints invalidate
old ranges rather than combining versions.

## Transport and concurrency

- **LocalTransport** calls the engine directly; local operation does not serialize through a child
  process.
- **PipeTransport** exercises the exact remote framing/server path through child stdio for reliable
  integration tests.
- **SshTransport** establishes one persistent control session and N persistent data sessions for the
  duration of a job. SSH stderr remains attached for host-key and authentication UX.
- **RsyncTransport** is the receiver-compatibility fallback. It launches the remote rsync executable
  in server mode over SSH and implements the compatible wire protocol locally; it does not require
  a local rsync executable and is not an alias for copying through SFTP or tar.
- The single-data-stream path is implemented first and remains the compatibility fallback.
- Multi-stream workers are coordination-free in memory, but share the receiver's durable staging and
  checkpoint state. No two streams write the same range.
- The CLI keeps `--streams 1..=16`; omitted `--streams` resolves through an evidence-backed policy.
  A provisional four-stream default may ship only if it improves paired performance on at least two
  materially different remote filesystems and causes no material regression on the validation host.
  Otherwise v1 defaults to one.

### Rsync protocol fallback

Automatic selection follows a strict capability chain:

1. Probe `xsync --server` and use the native xsync protocol when its version/capabilities match.
2. If and only if xsync is genuinely unavailable and installation/bootstrap is not possible or was
   declined, probe the remote `rsync --server` capability and negotiate a supported dialect.
3. If neither compatible receiver exists, fail before destination mutation with installation and
   `--transport` guidance.

Authentication failure, host-key failure, xsync protocol corruption, version mismatch, or a native
transfer that failed after mutation began must not silently fall back to rsync. Those are real
errors, and retrying through a different engine could hide a security/correctness problem. The CLI
offers `--transport=auto|xsync|rsync` so callers can require a specific path.

The fallback is a deliberately bounded compatibility implementation, not a promise to clone every
historical rsync extension. A research gate identifies the GNU rsync and openrsync protocol dialects
present on supported hosts, pins golden transcripts, and defines the v1 compatibility matrix.
Supported xsync operations map explicitly to rsync server arguments and protocol features.
Unsupported combinations fail before transfer rather than degrading silently.

Rsync fallback cannot claim xsync-native multi-stream striping, BLAKE3 frame verification, atomic
xsync staging, or xsync checkpoint resume unless independently implemented and proven for that
transport. Events and the final summary identify `transport=rsync`, negotiated protocol version,
remote implementation, mapped features, and unavailable guarantees. `--paranoid`, resume,
non-UTF-8 paths, delete, excludes, and metadata behavior each require explicit compatibility tests
or an early actionable rejection.

The required v1 fallback direction is local→remote, where the missing xsync process would have been
the receiver. Remote→local rsync-server fallback is added only after the sender dialect passes the
same compatibility suite. xsync never installs or uploads an executable merely because
`--transport=auto` was selected; any bootstrap/install step is separately visible and authorized.

## Compression and caching

Payloads use zstd level 3 by default when a bounded sample predicts a ratio below 0.95. The sample
decision is per logical payload or homogeneous batch, never based only on file extension. The
benchmark matrix compares 64 KiB, 256 KiB, and 1 MiB samples, compressible/incompressible/mixed data,
and small-frame overhead. `--no-compress` and `--compress-level` override the policy.

The checksum cache is an optimization, never an authority. Its key includes stable filesystem
identity, size, mtime, and ctime/change-time where available. Cache hits are accepted only after a
stable metadata read; corruption or schema mismatch rebuilds the cache. The per-run destination
index and persistent hash cache remain separate stores with separate lifetimes.

## Benchmark and decision policy

Benchmarking begins before the remote protocol is frozen and continues as a regression suite. Every
report records source revision, build profile, hardware, OS/kernel, filesystem, transport route,
corpus manifest digest, tool versions, stream count, compression policy, and individual samples.

Required controls:

- at least five repetitions, reporting median and MAD;
- deterministic, content-pinned corpora and an independent destination manifest oracle;
- paired ratios against baselines measured in the same run, with method order rotated or randomized;
- no comparison across different environments or corpora;
- noisy results are reported as unverified, never silently passed;
- correctness failures always fail, regardless of performance;
- warm-cache and first-pass results labeled honestly—never call the first repetition a cold-cache run;
- both `rsync -a` and a fair compressed rsync baseline where available;
- scan-only time, transfer time, verification time, wire bytes, CPU, and peak RSS reported separately.

Corpus/workload matrix:

- 100k×4 KiB flat files and a deeply nested real-shaped tree with the same entry count;
- one 10 GiB file, a mixed tree, incompressible media, and zero-byte-file storms;
- full initial copy, no-op second sync, 1% churn, metadata-only churn, type replacements, and deletes;
- local same-volume, local cross-volume, PipeTransport, native xsync over real SSH, and rsync-protocol
  fallback over real SSH;
- APFS plus at least two Linux destination filesystems; lossy/high-latency links when reproducible.

No README multiplier is published until the actual `xsync` protocol—not tar as a proxy—passes this
matrix. Absolute results remain machine-specific; release gates use comparable paired ratios.

## Implementation order

1. **Evidence harness and invariants** — benchmark/report schema, manifest oracle, scanner shapes,
   local clone spike, and remote baseline matrix.
2. **Finish the local engine** — stable source reads, reversible paths, bounded destination index,
   shared dispatch, local reflink/clone fast paths, and end-to-end CLI.
3. **Freeze protocol after spikes** — explicit limits, golden vectors, hostile-input tests, bounded
   framing/compression, and stable resume identities.
4. **Single-stream server path** — PipeTransport then real SSH, including push/pull and durable resume.
5. **Rsync receiver fallback** — research supported dialects, implement the bounded native client,
   and prove capability selection plus semantic differences.
6. **Multi-stream tuning** — add striping and choose a default only through the decision gate.
7. **Feature completion** — delete/exclude/dry-run/checksum cache/paranoid/JSONL and cloud-placeholder
   policy.
8. **Compression and UI** — measured sampling policy and terminal renderer.
9. **Release benchmarks and documentation** — reproducible reports, limitations, and only supported
   performance claims.

## v2 direction

- Daemon + native authenticated transport, services, tray/UI, and local event forwarding.
- FastCDC delta transfer gated by a network-versus-disk cost model.
- Filesystem event streams treated as hints; drop flags trigger targeted subtree reconciliation and
  full reconciliation runs only under appropriate idle/power policy.
- Optional persistent metadata index with explicit invalidation and reconciliation contracts.
- Hardlinks, xattrs/ACLs, sparse files, ownership, richer macOS metadata, and remote→remote.
- Platform-specific I/O such as `io_uring` or no-cache hints only after isolated benchmarks prove a
  benefit without correctness or cache-pressure regressions.
