# Story 8.1 — Release benchmark matrix: findings and release decision

**Status: both original release blockers are fixed and verified. 38 of 38 cells now pass
their correctness oracle. Two new defects surfaced by the same matrix remain open, and
tier expansion is still outstanding, so 8.1 is not yet closed.**

Every number here comes from a checked-in `xsync.bench.report.v1` document. Pre-fix
evidence is preserved under [`pre-fix/`](pre-fix/); current results are in
[`local/`](local/matrix.md) and [`ssh-mars/`](ssh-mars/matrix.md).

## How these runs were produced

[`benches/scripts/release-bench.py`](../../scripts/release-bench.py) runs, for each
corpus class / workload / route cell:

- **paired methods against a same-run baseline** — `xsync` and `xsync-rsync-transport`
  are candidates, reference `rsync -a` is the baseline, and `rsync -az` / `xsync-raw`
  are added where compression is a fair axis;
- **rotated method order** from `xsync-bench schedule`, so a candidate is never always
  measured after its baseline (`xsync-bench report` rejects an input whose ordering
  never crosses over);
- **per-invocation wall, CPU, and peak RSS** from `os.wait4` rusage for that exact child;
- **the independent manifest oracle after every single run**, locally via
  `xsync-bench verify` and remotely via `xsync-bench verify` on the receiver;
- **five repetitions**, emitted as `xsync.bench.input.v1` and rendered through
  `xsync-bench report`, so the Epic 0 median/MAD policy and `xsync-bench gate` apply
  unchanged. Rows whose MAD/median exceeds 15% are marked `noisy` and are not
  gate-able evidence.

Routes: local same-volume (APFS internal NVMe), cross-volume (APFS on external NVMe),
PipeTransport, and real SSH to `mars.local` (Arch Linux, ext4 on NVMe, GNU rsync
3.4.4 / protocol 32). Tier is `smoke` throughout.

## Blocker 1 (fixed) — stale mtime on unchanged directories

`mixed/content-churn` and `mixed/metadata-only-churn` failed the oracle on both local
routes, in all five repetitions, with four *directory* mtime mismatches per run;
`rsync -ac` passed the identical seeded state.

`finish_directories` in `crates/xsync-core/src/local.rs` iterated only
`directories.new` and `directories.changed`. A directory classified `unchanged` still
has its mtime bumped by the kernel when a child file is rewritten inside it, and
nothing restored it. The remote path never had this bug because its final metadata
pass already chained `directories.unchanged`.

The fix restores only directories actually mutated — the parents of every written,
created, or deleted entry — rather than sweeping every unchanged directory, which
would have added a syscall per directory to every no-op sync and cost the 1.98x no-op
win. A regression test,
`local::tests::rewriting_a_file_restores_its_unchanged_parent_directory_mtime`,
fails without the fix and passes with it.

## Blocker 2 (fixed) — no small-file batching, and stop-and-wait

`Message::FileBatch` carried exactly one entry (`entries: vec![rec]`) and the client
blocked for an `Ack` after both the batch frame and the segment frame, so every small
file cost two fully serialized round trips. The 32 MiB unacknowledged window
negotiated in the handshake was never used, and `strategy.rs` already implemented the
coalescing that plan.md specifies — the remote client simply never called it.

Both directions now coalesce small files (`SMALL_FILE_LIMIT`, `BATCH_TARGET_SIZE`,
`MAX_BATCH_FILES` from `strategy.rs`) into one metadata frame per batch, and write
frames without stopping for each acknowledgement. Replies are drained on a bounded
window (`MAX_PIPELINED_FRAMES = 256`) chosen so the peer's pending acknowledgements
stay near 10 KiB and always fit inside an ordinary pipe or SSH channel buffer — an
unbounded window deadlocks, because the receiver blocks writing acknowledgements once
its own buffer fills. The four metadata loops (directory creation, symlinks, deletes,
final directory metadata) are pipelined the same way.

### Result, deep-small over SSH (1,000 entries)

| | before | after | change |
|---|---:|---:|---:|
| xsync wall time | 8.731 s | **0.343 s** | **25.5x faster** |
| ratio vs `rsync -a` | 0.091 | **0.873** | 9.6x better |
| per-file cost | ~8.7 ms | ~0.34 ms | |

xsync is now at or above parity with `rsync -a` on four of five SSH cells:

| Cell | `rsync -a` | xsync | ratio |
|---|---:|---:|---:|
| deep-small initial-copy | 0.2993 s | 0.3429 s | 0.873 |
| flat-small no-op | 0.2334 s | 0.2275 s | **1.032** |
| compressible initial-copy | 0.7957 s | 0.7277 s | 1.096 *(noisy)* |
| incompressible initial-copy | 0.4747 s | 0.5958 s | 1.089 *(noisy)* |
| one-large-file initial-copy | 0.8820 s | 0.8783 s | **1.030** |

The pipe route improved in step: `mixed/initial-copy` 0.914 → **1.014**, `deep-small`
0.697 → **0.903**, `content-churn` 1.108 → **1.472**.

This matches the independent finding in `~/projects/f2/BENCHMARKS.md` §6, where a
per-file round-trip protocol ran at 47 files/s against 6,007 for a single framed
stream — framing worth 20–80x, parallel streams worth only 1.0–1.6x on top. That
supports keeping stream count a tunable rather than an architectural concern, and it
means Story 4.2's multi-stream work was never where the headline win lived.

## Two additional correctness bugs found and fixed

- **Type-replaced directories were never created remotely.** The push client sent only
  `plan.directories.new`; a source directory whose destination holds a file is
  classified `changed`, so it was silently skipped and `mixed/type-replacement` failed
  the oracle with a kind mismatch. The local path already chained both buckets.
- **Type-replaced directories left their parent stale locally.** The Blocker 1 fix
  initially noted parents of `directories.new` only; a replaced directory lands in
  `changed`, so its parent's bumped mtime went unrestored.

## Fixed: `--checksum` was 63x slower than rsync on the local path

**Before:**

| Cell | `rsync -ac` | `xsync --checksum` | ratio |
|---|---:|---:|---:|
| content-churn same-volume | 0.0645 s | 4.0888 s | **0.016** |
| content-churn cross-volume | 0.1198 s | 3.7814 s | 0.032 |
| content-churn pipe | 0.0959 s | 0.0664 s | 1.472 |

Isolated at 4.31 s wall against 0.29 s CPU — blocking, not computing, at about 9 ms per
file. `HashCache::hash_file` opened and committed a separate redb write transaction per
cache miss, and redb commits durably by default, so every insert cost an fsync. The pipe
route was unaffected because it does not take this path.

The durability default was inherited, not chosen: `Durability` appears nowhere else in the
codebase, and "durable" is used only for the resume journal, which genuinely requires it.
The hash cache is explicitly rebuildable.

**Fix.** Digests are buffered in memory and committed in batches — once per run for any
tree under 4,096 files, and on drop. `Durability::Eventual` is used rather than
`Durability::None`: redb only frees pages at durability levels above `None`, so committing
exclusively at `None` would grow the cache file without bound. The per-file 1 MiB read
buffer is now sized from the fingerprint the caller already holds, avoiding both the
allocation and an extra `stat`.

**After** (11 repetitions, both methods inside the 15% noise policy):

| Cell | `rsync -ac` | `xsync --checksum` | ratio |
|---|---:|---:|---:|
| content-churn same-volume | 0.1127 s | **0.2146 s** | **0.487** |

Wall time 4.0888 s → 0.2146 s, **19x faster**, and the paired ratio improves 30x from
0.016 to 0.487. A second run over an unchanged tree reuses the cache and completes in
0.11 s.

xsync remains about 2x slower than `rsync -ac` here, but that is no longer anomalous — it
now matches xsync's ordinary local per-file overhead on this corpus (mixed initial-copy
0.534, deep-small 0.578) and is subsumed by TUNING-TASKS Epic T1 rather than being a
distinct defect.

A regression test, `hash_cache::tests::buffered_digests_survive_drop_and_reopen`, deletes
the source files after dropping the cache so that a returned digest can only have come from
the committed database; it fails if the flush on drop regresses.

Evidence: [`checksum-fix/`](checksum-fix/). The cross-volume row could not be re-measured —
`/Volumes/XSYNC_BENCH` was unmounted at the time of the re-run — and remains outstanding.

## Open: content verification is a tautology without `--paranoid`

In `run_sink`, small and medium files are verified as
`let hash = blake3::hash(&data); write_file_with_retry(&entry, &hash, |_| Ok(data.clone()))`
— the expected hash is computed from the received buffer, so the comparison always
passes. Only the declared length is genuinely checked end to end. The sender does
compute a real digest (`StableRead.blake3`), but it reaches the receiver only in
`LargeFileFinish`, and `finish_large` compares it only under `--paranoid`.
`EntryRecord.fingerprint` cannot simply be reused: it carries device and inode identity
for resume and `--checksum` classification. Closing this likely needs a protocol
version bump.

## Where xsync wins today

Comparable rows, current run:

| Cell | Route | Ratio |
|---|---|---:|
| one-large-file initial-copy | same-volume | **2.541** |
| mixed no-op-second-sync | same-volume | **1.980** |
| compressible initial-copy | same-volume | **1.493** |
| mixed content-churn | pipe | **1.472** |
| incompressible initial-copy | same-volume | **1.396** |
| mixed type-replacement | same-volume | **1.297** |
| flat-small no-op | ssh | **1.032** |
| one-large-file initial-copy | ssh | **1.030** |

The remaining consistent loss is local many-small-files: `deep-small` same-volume sits
at 0.578 with 2.46 s of CPU against rsync's 0.23 s — 10.6x. That is CPU-bound, not
round-trip bound, and is a separate profiling problem from Blocker 2.

## Compression policy — supported by evidence

Pipe route, unchanged by this work:

| Corpus | xsync wire bytes | `--no-compress` wire bytes | Ratio |
|---|---:|---:|---:|
| compressible | 2,944 | 2,098,816 | **713x smaller** |
| incompressible | 2,098,816 | 2,098,816 | 1.00 — correctly skipped |
| mixed | 915,733 | 1,794,073 | 1.96x smaller |

## Still missing

- **Regression and full tiers.** Everything here is `smoke`. The SSH path is no longer
  the blocker it was, so the 100k-entry regression corpus is now schedulable.
- **`mixed` over SSH.** macOS stores symlink permission bits and Linux forces 0777, so
  the oracle cannot be satisfied cross-platform; `rsync -a` fails identically,
  confirming a platform limit rather than a tool defect.
- **`freya.local` (rsync 3.5.0)** as a second reference receiver — it has no Rust
  toolchain, so no native `xsync` or remote oracle.
- **A `tar` reference row**, listed as optional in the acceptance criteria.
- **A checked-in gate baseline.** These reports can serve as one once the tiers above
  are run.

## Decision

Story 8.1 remains open but is no longer blocked on the two defects that made a release
green check impossible. Closing it needs the regression tier, a nominated gate
baseline, and a decision on the two open defects above. Stories 8.2 and 8.3 stay
downstream.

No performance claim should be published except the comparable cells named here, each
with its corpus, route, baseline command, and report link attached.
