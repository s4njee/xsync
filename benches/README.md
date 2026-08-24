# xsync benchmark foundation

`xsync-bench` is the correctness and reporting layer for the benchmark stories in
[`tasks.md`](../tasks.md). It is a separate workspace package and deliberately does not depend on
`xsync-core`, so its filesystem oracle cannot reproduce an engine scanner bug.

Build the release harness before recording performance:

```bash
cargo build --release -p xsync-bench
```

## Deterministic corpora and workload states

> **Legacy for performance purposes.** These synthetic classes remain the **correctness**
> fixtures — they are reproducible, they pin edge cases such as zero-byte storms and
> non-UTF-8 names, and the manifest oracle depends on them. They are no longer the basis
> for performance claims: they are the wrong shape (synthetic flat corpora enumerate
> roughly 10x faster than real trees, which are directory-open bound), the wrong scale
> (smoke tier is 513 items / 1.77 MB, where process startup dominates), and they hide
> both wins and losses. Performance work is tuned against the real corpora defined
> in [`TUNING.md`](../TUNING.md). Use these classes to prove a change is *correct*, and
> the real corpora to prove it is *faster*.


Create a fixture only through a marker-owned scratch run:

```bash
target/release/xsync-bench corpus \
  --base /path/to/benchmark-scratch \
  --class deep-small \
  --tier regression \
  --workload content-churn \
  --seed 42
```

The command prints the owned run root. It contains `source/`, the prepared `destination/`,
`source.manifest.json`, `destination-initial.manifest.json`, and `scenario.json`. The scenario pins
the generator schema, seed, resolved sizes, workload, manifest identities, and changed/partial item
counts. `entry_count` describes entries below the source root; the independent manifest also counts
the root itself.

Seven classes cover filesystem topology and data behavior:

| Class | Purpose |
|---|---|
| `flat-small` | 100,000 flat files in regression/full tiers |
| `deep-small` | 100,000 total entries across deterministic ten-level branches in regression/full tiers |
| `zero-byte-storm` | Empty-file create/enumeration pressure |
| `mixed` | Directories, empty/small compressible and pseudo-random files, plus symlinks on Unix |
| `compressible` | Uniform seeded repetitive payloads |
| `incompressible` | Uniform deterministic pseudo-random payloads |
| `one-large-file` | 8 MiB smoke, 1 GiB regression, and allocated 10 GiB full-tier payload |

`--entry-count` and `--large-file-bytes` provide explicit test overrides. Full-tier generation is
never implicit; `smoke` is the default and is sized for ordinary CI.

Every class can be prepared as `initial-copy`, `no-op-second-sync`, `content-churn`,
`metadata-only-churn`, `type-replacement`, `delete`, or `interrupted-resume`. Churn, replacement,
and delete select a stable seed-dependent one percent of eligible files (at least one for tiny
overrides). Content churn preserves size and normalized metadata for nonempty files; metadata churn
does not alter bytes. The interrupted state contains a deterministic half of the leaf entries plus
a named staging artifact, allowing a resume benchmark to distinguish restart from a fresh copy.

Two generations with the same schema, seed, class, and sizing have identical source manifest
digests. Changing the seed changes bytes or normalized metadata, including for a zero-byte corpus.
The fixture self-tests exercise that invariant across all seven classes.

## Scanner and planner evidence

The engine runner is a separate package so the independent oracle remains free of `xsync-core`:

```bash
cargo build --release -p xsync-engine-bench
target/release/xsync-engine-bench run \
  --root /owned/run/source \
  --shape deep-small-100k \
  --repetitions 5 \
  --memory-budget-mib 512 \
  --json scanner-plan.json \
  --markdown scanner-plan.md
```

Each repetition runs in a fresh process. Reports retain both scan passes, their combined
syscall-sensitive time and entries/s, destination-`HashMap` construction, planner classification,
true process peak RSS, and the scanner result queue's producer-observed high-water mark. Linux reads
the kernel's `VmHWM`; macOS uses `/usr/bin/time -l`. A report requires at least five repetitions and
an independent corpus manifest.

The checked-in Story 0.3 evidence and platform decision are in
[`results/story-0.3/DECISION.md`](results/story-0.3/DECISION.md).

## Remote framing and baseline evidence

Story 0.5 uses a benchmark-only remote framing spike because production SSH transport is scheduled
for Stories 4.2–4.5. Build it and run a real SSH matrix with an independently generated manifest:

```bash
cargo build --release -p xsync-engine-bench --bin xsync-remote-spike
python3 benches/scripts/remote-matrix.py \
  --source /owned/run/source \
  --manifest /owned/run/source.manifest.json \
  --host user@receiver \
  --remote-binary /absolute/remote/xsync-remote-spike \
  --destination-base /remote/owned/run/matrix \
  --filesystem ext4 \
  --profile native \
  --methods rsync-a,rsync-az,xsync-1,xsync-2,xsync-4,xsync-8,xsync-adaptive-1 \
  --repetitions 5 \
  --json remote.json \
  --markdown remote.md
```

The native spike accepts flat regular-file corpora only. It opens persistent SSH data sessions,
uses bounded frames, verifies each file with BLAKE3 before atomic publication, and can apply an
adaptive zstd level-3 policy. The runner rotates and reverses method order, records setup and
application-wire counts, measures process RSS, and rejects any sample whose remote manifest differs.
Reference rsync and the future native rsync-protocol fallback are separate capability rows.

Probe compression sampling independently with:

```bash
target/release/xsync-remote-spike compression-probe \
  --corpora short=/owned/short/source \
  --corpora compressible=/owned/compressible/source \
  --corpora incompressible=/owned/incompressible/source \
  --corpora mixed=/owned/mixed/source \
  --sample-bytes 65536,262144,1048576 \
  --json compression.json \
  --markdown compression.md
```

The checked-in default decision and authoritative reports are in
[`results/story-0.5/DECISION.md`](results/story-0.5/DECISION.md).

## Local clone/reflink evidence

The Story 0.4 runner compares a physical buffered copy with a capability-gated local clone. It
rotates method order, stages beside the final target, and independently verifies each output after
the operation timer stops:

```bash
cargo build --release -p xsync-engine-bench
target/release/xsync-clone-bench \
  --source /owned/run/source/large.bin \
  --destination /same-filesystem/xsync-clone-target \
  --kind file \
  --repetitions 5 \
  --json clone.json \
  --markdown clone.md
```

Use `--kind directory` only for a complete fresh tree. `--paranoid` adds staged and final-name
content readback to the candidate operation. The report always uses the independent Story 0.1
manifest oracle, even without `--paranoid`. Checked-in results and the selection/defer decision are
in [`results/story-0.4/DECISION.md`](results/story-0.4/DECISION.md).

## Release benchmark matrix (Story 8.1)

`release-bench.py` is the release-evidence runner. Unlike a smoke matrix it produces
*gate-able* results: every cell carries a same-run `rsync -a` baseline, rotated method order,
per-invocation wall/CPU/peak-RSS from `os.wait4` rusage, an independent oracle verification after
every run, and an `xsync.bench.input.v1` document rendered through `xsync-bench report`.

```bash
cargo build --release -p xsync -p xsync-bench
python3 benches/scripts/release-bench.py \
  --routes same-volume,cross-volume,pipe \
  --cross-volume /Volumes/XSYNC_BENCH/release-bench \
  --repetitions 5 --tier smoke \
  --out /tmp/xsync-release
```

Routes are `same-volume`, `cross-volume`, `pipe` (a child `xsync --server` over stdio, with an
equivalent `rsync` rsh wrapper so both tools cross a process boundary), and `ssh`. The `ssh` route
needs a receiver with a native `xsync` build plus `xsync-bench` for the remote oracle:

```bash
python3 benches/scripts/release-bench.py \
  --routes ssh --repetitions 5 --tier smoke \
  --cells deep-small:initial-copy,one-large-file:initial-copy \
  --ssh-host user@receiver \
  --ssh-destination /home/user/xsync-release-bench \
  --remote-bin-dir /home/user/xsync-build/target/release \
  --remote-bench /home/user/xsync-build/target/release/xsync-bench \
  --ssh-filesystem "ext4 on NVMe" \
  --out /tmp/xsync-release-ssh
```

The ssh route adds production `xsync --transport rsync` as its own row, so the native transport and
the `RsyncTransport` fallback are always separately visible. `--cells` overrides the default
class:workload list.

Each cell emits `input-<cell>.json` (every repetition, unaggregated), `report-<cell>.json`, and
`report-<cell>.md`; the run emits `matrix.json` and `matrix.md`. The matrix marks a row `noisy` when
it or its baseline exceeds the Epic 0 15% MAD/median policy — noisy rows are reported but are not
gate-able evidence.

Corpora containing symlinks (the `mixed` class) cannot satisfy the manifest oracle across a
macOS source and a Linux receiver, because macOS stores symlink permission bits while Linux forces
0777; `rsync -a` fails the identical check. Use symlink-free classes for cross-platform ssh routes.

Checked-in results and the release decision are in
[`results/story-8.1/DECISION.md`](results/story-8.1/DECISION.md). The real-world corpora and
the optimization spikes they support are defined in [`TUNING.md`](../TUNING.md).

## Independent manifests

The manifest includes the inspected root and every descendant. It pins native path-component bytes,
object kind, logical length, BLAKE3 content, permission/special mode bits, nanosecond mtime, and raw
symlink target. Symlinks are never followed.

```bash
target/release/xsync-bench manifest SOURCE --out expected-manifest.json
target/release/xsync-bench verify DESTINATION --manifest expected-manifest.json \
  --json verification.json
```

`verify` exits nonzero for missing/unexpected paths or any content, type, mode, mtime, or symlink
target difference. Details are bounded in JSON, while the total mismatch count is retained.

## Measurement order

Generate a deterministic rotated order instead of always running the candidate after its baseline:

```bash
target/release/xsync-bench schedule \
  --methods rsync-a,xsync \
  --repetitions 5 \
  --out schedule.json
```

The report builder independently rejects paired inputs whose ordering never crosses over.

## Reports

A raw `xsync.bench.input.v1` document contains:

- `build`: source revision, unique release build ID, and profile;
- `environment`: hardware, OS, kernel, destination filesystem, transport, and route;
- `session`: stream count and compression policy;
- `corpus`: fixture schema, 64-character lowercase BLAKE3 manifest digest, and description;
- `tools`: exact versions and secret-free command descriptions;
- `results[]`: method name, optional same-run baseline, and every sample.

Every sample records repetition/pairing ID, actual method order, wall and CPU seconds, peak RSS,
item/logical/wire counts, timestamped scan/plan/transfer/metadata phase timings, cache state,
and the independent oracle result.
Allowed cache labels are:

- `first_pass`: first observation, with no claim that the kernel cache was empty;
- `warm`: intentionally warmed;
- `cold_evicted`: only valid with a nonempty `cache_eviction_method` describing the real action.

Create both versioned artifacts in one validated operation:

```bash
target/release/xsync-bench report \
  --input raw-samples.json \
  --json report.json \
  --markdown report.md
```

The report retains every repetition and adds medians, median absolute deviations, per-phase medians,
peak RSS, median wire bytes, and same-repetition baseline/candidate speedups.

## Regression gates

```bash
target/release/xsync-bench gate \
  --current current.json \
  --baseline baseline.json \
  --strict \
  --json gate.json
```

Correctness failures always fail. A gated result needs at least five repetitions. Performance is
compared only when report schema, full environment, session configuration, and content-pinned corpus
match. Wall or paired-ratio MAD/median above 15% is reported as unverified. A paired speedup may
degrade by at most 15%. Absolute wall time is never used as a historical release gate.

Without `--strict`, a missing or incomparable historical report is advisory. With `--strict`,
zero performed comparisons fails, preventing an empty green CI check.

## Owned scratch

```bash
target/release/xsync-bench scratch-create --base /path/to/benchmark-scratch
target/release/xsync-bench scratch-clean \
  --root /path/to/benchmark-scratch/run-ID \
  --base /path/to/benchmark-scratch
```

Cleanup accepts exactly one direct, marker-owned child of the expected canonical base. It refuses
the base itself, filesystem root, home, current repository, nested paths, escaped paths, symlink
markers, missing markers, and tampered markers.

Story 0.3 supplies the process runner and platform metrics. Stories 0.1 and 0.2 own the report
schema, oracle, scheduling, scratch safety, corpus definitions, and workload states it consumes.
