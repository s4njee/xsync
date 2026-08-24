# xsync v2 tuning — Epics, Stories & Acceptance Criteria

Execution plan for the optimization work defined in [TUNING.md](TUNING.md). Written so an
agent with no prior context can pick up any story and execute it. Corpus definitions,
measured baselines, and the reasoning behind each spike live in TUNING.md; this file is
the work breakdown.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

---

## Rules for any agent working from this file

These are invariants, not suggestions. Violating any of them produces a result that looks
valid and is not.

1. **Corpora are source-only.** Never pass a corpus root as a destination to any tool, and
   never write inside one. `Docker.raw` *is* the user's live Docker state — every
   container, image, and volume. A stray destination argument destroys real data.
2. **Never mutate a corpus.** Do not run `docker system prune`, `fstrim`, `git gc`, package
   installs, or builds inside a corpus. If a corpus needs to change, that is a decision for
   the user, and every pinned digest must be recaptured afterwards.
3. **Docker must be stopped for corpus D.** Verify with `pgrep -x "Docker Desktop"` and
   `pgrep -f com.docker.backend` before any run. A running hypervisor mutates the image
   mid-measurement.
4. **Never present a sampled oracle as a full one.** If verification samples, the report
   must record that it sampled and what fraction.
5. **Never compare across differing source digests.** A live tree can change between
   sessions. Digest mismatch invalidates the comparison; it does not warrant a footnote.
6. **Never claim a speedup from a row whose MAD/median exceeds 15%.** The harness marks
   these `noisy`. Noisy rows are reported, never gated on.
7. **Never verify sparseness on a compressing filesystem.** ZFS with `compression=zstd`
   stores an all-zero record as a hole, so a destination looks correctly sparse even when
   the sender wrote every zero. Sparse correctness is verified on ext4 (`mars`) or APFS
   (local cross-volume) only. See T2.
8. **Report what happened.** A failed run, an ENOSPC, a skipped route, or a corpus that
   drifted is a result. Record it as a failed or blocked row with its error text.

---

## Environment

| Host | Role | Details |
|---|---|---|
| local (this Mac) | source, and local routes | Apple M1 Max, 10 cores, 64 GB, APFS. `/` 3.6 TiB with ~1.1 TiB free |
| `/Volumes/XSYNC_BENCH` | cross-volume destination | APFS on external NVMe, 238 GB total |
| `sanjee@mars.local` | SSH receiver | Arch Linux, ext4 on NVMe, GNU rsync 3.4.4/protocol 32, 614 GB free, 24 cores. Cargo at `~/.cargo/bin/cargo`; build tree at `~/xsync-8.1-build` |
| `sanjee@freya.local` | secondary receiver | CachyOS, ZFS raidz2 (12×6 TB) at `/mnt/raid6`, 9.9 TB free, rsync 3.5.0, 32 cores. **No Rust toolchain.** `compression=zstd`, pool 86% full, measured **56 MB/s** over the link |

`freya` is not a general-purpose benchmark receiver. 56 MB/s puts a 130 GB transfer at
~40 minutes per repetition, and its ZFS compression makes it unusable for sparseness
verification. Use `mars` unless a story names `freya` specifically.

Existing harness: [`benches/scripts/release-bench.py`](benches/scripts/release-bench.py)
(paired methods, rotated order, `os.wait4` rusage, manifest oracle, `xsync.bench.input.v1`
output through `xsync-bench report`). Read
[`benches/README.md`](benches/README.md) before changing it.

---

## Epic T0 — Harness prerequisites

Everything else depends on this epic. The current runner only understands the synthetic
generator; it cannot address a real corpus, cannot afford to verify a 1.3M-file
destination five times per cell, and does not record the quantities the later epics need.

### Story T0.1 — Real-corpus registry
- [x] Add named real corpora to the runner alongside the legacy `xsync-bench corpus`
  generator: `congress-1k|10k|100k|1m`, `manga`, `cb7`, `docker-raw`. Each entry carries a
  source path, a pinned manifest digest, and a workload list it supports.

**AC**
- `--corpus congress-10k` runs without a `--cells` class:workload pair, and the report's
  `corpus.description` names the corpus and its pinned digest.
- Corpus paths resolve to the definitions in TUNING.md §2 — for example `congress-10k` is
  `~/projects/csearchv2/congress/data/100` (11,280 files), `congress-1m` is
  `~/projects/csearchv2/congress/data` (1,318,771 files).
- The runner **refuses** to start if a resolved destination path is inside any corpus root,
  with a clear error naming the corpus. A unit test covers this refusal.
- Real corpora have no pinned `destination/` template, so `initial-copy` seeds an empty
  destination and other workloads are rejected until T0.7 provides mutation states.
- Legacy synthetic classes keep working unchanged.

**Results:**
- Added a real-corpus registry to `benches/scripts/release-bench.py` for `congress-1k`,
  `congress-10k`, `congress-100k`, `congress-1m`, `manga`, `cb7`, and `docker-raw`. Paths
  default to the project-local `corpora/` directory and can be overridden with
  `XSYNC_CORPORA_DIR`.
- Added `--corpus NAME` selection, including the two-subtree staging required by
  `congress-1k`. Real corpora expose the initial-copy and locally derived mutation workloads;
  unsafe remote mutation workloads are rejected rather than given a fabricated destination
  state.
- Real-corpus runs generate an independent manifest and include its digest and corpus name in
  the report description. Existing synthetic `--cells` selection remains unchanged.
- Added a preflight destination containment check that names the corpus and refuses any
  destination inside a registered corpus root. Python syntax, registry resolution, and the
  containment refusal check pass.
- Added the documented regular-file counts to each real-corpus definition and made setup
  reject a layout mismatch before benchmarking. Focused runner tests now cover timestamped
  phase parsing, deterministic destination mutation, source protection, destination safety,
  registered counts, and digest-drift reporting; all five pass.
- Completed a five-repetition same-volume smoke cell with `--corpus congress-10k`. The
  independent oracle passed every repetition with digest
  `f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`; median wall time was
  2.736 s for `rsync -a` and 6.480 s for `xsync`, with both rows below the 15% noise limit.
- Completed the full local smoke run over same-volume and PipeTransport routes: 2/2 cells passed,
  all 50 destination verifications passed, and every method row remained below the 15% noise
  threshold. Median paired ratios were `xsync` 0.479 on same-volume, `xsync` 0.642 over pipe,
  and `xsync-raw` 0.659 over pipe against `rsync -a`; `rsync-az` over pipe measured 0.970.
- Corrected the registry to resolve both supported layouts: the project-local `corpora/congress`
  tree and the live `csearchv2/congress/data` tree selected through `XSYNC_CORPORA_DIR`.
- Captured and pinned read-only manifests for congress-1k (combined staged source), congress-10k,
  congress-100k, congress-1m, manga, and cb7. The live `docker-raw` image remains intentionally
  absent from the checkout.
- Re-ran the real `congress-10k` registry smoke with the live source: one cell passed, all five
  repetitions passed the independent oracle, and every repetition matched the pinned digest.

**Next Steps:**
- Add a disposable CLI-level regression test for the full `--corpus congress-10k` path; the
  existing focused registry and containment tests plus the retained smoke report cover the
  behavior operationally.

### Story T0.2 — Oracle cost control at scale
- [x] Verification of a 1.3M-file / 14 GB destination cannot run at full strength five
  times per method per cell. Add a sampling mode to `xsync-bench verify` and record the
  mode in the report.

**AC**
- `verify --sample <fraction>` checks all entries' metadata and a deterministic,
  seed-reproducible subset of content hashes.
- The verification record carries `mode: full|sampled`, the fraction, and the seed. A
  sampled verification is never labelled or summarised as full anywhere in JSON or Markdown.
- At least one repetition per cell runs `full`; sampling is only permitted for the rest.
- `congress-1m` full verification time is measured and recorded so the sampling decision is
  evidence-based rather than assumed.

**Results:**
- Added `xsync-bench verify --sample FRACTION --sample-seed SEED`. It always checks every
  entry's metadata, but hashes only a deterministic BLAKE3-selected fraction of regular files.
- Sampled results now explicitly record `mode: sampled`, the fraction, the seed, and the number
  of file contents hashed. They are not presented as full verification.
- The release runner now accepts `--verify-sample FRACTION`; repetition 1 is forced to full
  verification and later repetitions use sampling only when that flag is supplied. Markdown
  labels sampled oracle rows as sampled.
- Added unit coverage for repeatability and metadata mismatch detection. `cargo test -p
  xsync-bench` passes.
- Measured full verification against the live `congress-1m` source: 1,787,546 manifest items,
  1,318,771 regular files, 10,987,200,342 logical bytes, zero mismatches, and 282.17 seconds
  wall time. The manifest capture itself took 292.49 seconds. The result is retained at
  `/tmp/xsync-t0-congress-1m-verify.json`.
- The evidence supports keeping sampling opt-in rather than silently reducing correctness; the
  runner forces repetition 1 to full verification whenever later repetitions are sampled.

**Next Steps:**
- Use the measured full cost when planning 1m release matrices; no fixed sampling fraction is
  imposed without a workload-specific accuracy requirement.

### Story T0.3 — Source drift detection
- [x] Capture the source manifest digest at the start of every run and compare it to the
  corpus's pinned digest.

**AC**
- A mismatch fails the cell with `status: drifted`, reporting both digests. It never
  silently proceeds.
- The digest is recorded per repetition, not once per cell, so mid-run mutation is caught.
- For `docker-raw`, the runner additionally asserts Docker is not running before the first
  repetition and fails with an actionable message if it is.

**Results:**
- The registry now accepts authoritative pinned digests, with the verified `congress-10k`
  digest checked in and the remaining real-corpus digests supplied explicitly through
  `XSYNC_CORPUS_DIGEST_<NAME>` until they are approved.
- A real-corpus run is refused when its digest is missing, malformed, or different from the
  pinned value. The runner rechecks the source before every method invocation and records the
  observed digest on that repetition.
- Digest mismatches become a distinct `status: drifted` matrix row containing both expected and
  observed digests; they are not reported as performance failures.
- `docker-raw` now performs the required Docker Desktop/backend process check before starting.
- Python syntax validation and a focused drift-reporting test pass. The new disposable source
  mutation test manifests a temporary source, changes one file, manifests again, and asserts the
  distinct drift failure.

**Next Steps:**
- Exercise the rendered drifted matrix-row path through a full CLI run if a future benchmark
  needs that specific report artifact; the per-repetition guard is now tested.

### Story T0.4 — Phase-level timing
- [x] The report schema already carries `phases_seconds`, but the runner populates only
  `seed_destination`, `transfer`, and `verify_oracle`. Epics T1 and T7 need the transfer
  itself decomposed.

**AC**
- xsync runs record `scan`, `plan`, `transfer`, and `metadata` phases separately, sourced
  from `--progress-json` events rather than inferred.
- Phase medians appear in `report-<cell>.json` and in the Markdown table.
- Phases sum to within 5% of measured wall time, or the discrepancy is recorded as
  `unaccounted`.

**Results:**
- xsync now emits timestamped `phase` progress-json events for scan, plan, transfer, and
  metadata boundaries in local, native push, and native pull paths. The runner consumes those
  events and keeps seeding/oracle time separate from the transfer phase budget.
- Report generation now adds an explicit `unaccounted` phase when supplied phase timings differ
  from measured wall time by more than 5%, so a discrepancy cannot disappear silently.
- Local pipeline tests assert that all four phase start/end pairs are emitted. Native server
  integration tests and the benchmark crate test suite also pass.
- Retained a five-repetition real `congress-10k` report at
  `benches/results/tuning/T0.4/README.md`: xsync phase medians were scan 0.222 s, plan 0.037 s,
  transfer 5.655 s, and metadata 0.00006 s, with the report carrying the phase medians in both
  JSON and Markdown.

**Next Steps:**
- Use the same phase-bearing schema for later SSH and worker-policy reports.

### Story T0.5 — Allocated-bytes accounting
- [ ] Sparse transfer makes "bytes transferred" ambiguous. The event schema distinguishes
  logical, physical, and wire bytes; a fourth quantity is required.

**AC**
- `allocated_bytes` is recorded for source and destination, obtained from the extent map
  rather than `stat` size.
- For `docker-raw` the report shows apparent 3,721.9 GB, allocated ~130.2 GB, and they are
  never conflated in any throughput calculation.
- Throughput for sparse corpora is computed against allocated bytes, and the Markdown
  states which basis was used.

**Results:**
- The independent manifest now walks `SEEK_DATA`/`SEEK_HOLE` extents on Unix and falls back to
  filesystem allocation blocks when the filesystem does not support extent queries. Allocation
  remains outside the content digest so moving a corpus between filesystems does not create
  false source drift.
- Verification records destination allocated bytes, and each benchmark sample records source
  and destination allocation separately from logical and wire bytes. Markdown repetitions show
  the allocation basis directly.
- Reports now derive allocated-byte throughput from median source allocation divided by median
  wall time and label that basis in Markdown.
- This is implemented and covered by the benchmark test suite, but no `docker-raw` measurement
  was run because that live corpus is not present in this checkout.

**Next Steps:**
- Add an explicit recorded `allocation_method`/fallback warning to the manifest and report, then
  validate the extent walk against `docker-raw` on ext4 `mars`.
- Validate the implementation on ext4 `mars`; do not use `freya` for sparse correctness.

### Story T0.6 — Cold-cache and bandwidth-limited routes
- [ ] Every measurement to date is warm-cache over an unconstrained link, which is the one
  regime where xsync's compression and dedup advantages cannot appear.

**AC**
- A cold-cache mode performs a real eviction (`purge` on macOS, `drop_caches` on Linux) and
  records `cache_state: cold_evicted` with the eviction method, per the Epic 0 schema. It
  must never label a first repetition as cold.
- A bandwidth-limited route shapes to 50, 100, and 1000 Mbit with configurable added
  latency, and records the shaping parameters in `environment`.
- Both modes are selectable per cell and recorded in the report.

**Results:**
- Added `--cache-state cold` for local routes. It performs macOS `purge` or Linux
  `sync + drop_caches` before repetitions after the first, records the real method as
  `cold_evicted`, and leaves repetition 1 labelled `first_pass`.
- SSH cold-cache runs are rejected because the receiver's cache state cannot be established by
  the local runner. No bandwidth-shaped measurement exists yet.
- Added opt-in macOS SSH shaping with `--bandwidth-mbit 50|100|1000` and `--latency-ms`.
  It resolves the receiver, configures a dedicated dummynet pipe and PF anchor, records the
  exact shaping description in report environment metadata, and cleans up the pipe/anchor in a
  `finally` block. Shaping is refused for local/pipe routes and unsupported hosts.
- Python syntax validation passes; an actual cold run remains environment-dependent because it
  requires host eviction/network privileges. No shaped transfer was run in this checkout.
- Retried against reachable ext4 `mars` at `192.168.1.119`: the shaped cell was recorded as
  blocked with the exact error `dnctl: socket: Operation not permitted`. A local cold-cache smoke
  likewise recorded the exact `purge` failure `Unable to purge disk buffers: Operation not
  permitted` after the first full repetition completed successfully. No run was labelled cold.

**Next Steps:**
- Re-run with macOS cache-eviction and dummynet/PF privileges; the harness now records these
  failures without fabricating cold or shaped evidence.

### Story T0.7 — Mutation states for real corpora
- [ ] Real corpora have no pinned "previous version" the way generated fixtures do, so
  `no-op`, `content-churn`, and `delete` workloads need a derivation rule.

**AC**
- `no-op-second-sync` is produced by syncing once, then measuring the second sync. No corpus
  mutation is involved.
- `content-churn` and `delete` mutate the **destination**, never the source, by a
  deterministic seeded selection of 1% of entries.
- The seeded selection is reproducible across runs and recorded in the report.

**Results:**
- `--corpus NAME --workload initial-copy|no-op-second-sync|content-churn|delete` is now exposed
  by the runner. The source is never mutated: no-op seeds a destination once, while churn and
  delete copy the source and mutate only the destination.
- Churn/delete choose a deterministic 1% of destination files from a seed-derived ranking, and
  each sample records the selected relative paths. The remote route is explicitly blocked for
  these real-corpus mutations until remote destination preparation is implemented.
- The implementation is syntax-checked, focused runner tests pass, and the benchmark crate
  tests pass. A real-corpus end-to-end mutation run is still blocked here because the pinned
  real corpora are absent.

**Next Steps:**
- Add a disposable real-corpus integration test that runs all three derived workloads twice and
  asserts identical selected paths, unchanged source digest, and successful final oracle.
- Add remote destination preparation with the same recorded selection before enabling SSH
  mutation measurements.

---

## Epic T1 — Close the per-file syscall gap

The highest-return work and a prerequisite for every local performance claim. Measured on
`congress-10k` (11,280 files, 112 MB, local same-volume, initial copy):

| | wall | user | sys | CPU |
|---|---:|---:|---:|---:|
| `rsync -a` | 3.75 s | 0.32 s | 4.58 s | 4.90 s |
| `xsync` | 7.20 s | 7.52 s | 21.37 s | **28.89 s** |

1.9 ms of kernel time per file against rsync's 0.41 ms — **while copying zero bytes**, since
the APFS clone path engaged and the run reported `0 physical`. With byte movement already
eliminated, the entire remaining cost is per-file syscall volume.

### Story T1.1 — Syscall attribution
- [~] Produce a per-file syscall histogram for both tools and attribute the delta.

**AC**
- A reproducible trace on `congress-1k` yields counts by syscall for `xsync` and `rsync -a`,
  checked in under `benches/results/tuning/T1/`.
- The write-up names the specific syscalls responsible for the majority of the gap, with
  counts, not adjectives.
- Method is documented well enough to re-run; if `dtruss` is blocked by SIP, the fallback
  used is stated.

**Blocker:** A retry after Full Disk Access was enabled produced the same result:
`dtrace: system integrity protection is on, some features will not be available` followed by
`dtrace: failed to initialize dtrace: Operation not permitted`. `fs_usage` still reports
`must be run as root`, and `sudo` is unavailable in this environment. Full Disk Access therefore
does not provide the root/SIP entitlement required for this histogram. A privileged trace run or
a user-supplied trace capture is needed; aggregate wall/CPU timings are not a substitute.

The user-run privileged capture is stored in
`benches/results/tuning/T1/syscall-trace-20260824-094017/`. Both xsync and rsync completed and
each disposable destination contains 11,280 files, but both `dtruss` reports contain only the SIP
warning and an empty `CALL / COUNT` table. No syscall histogram was emitted, so this is valid
transfer evidence but not a completed attribution trace.

**Current result:** The attempted trace and the exact errors are recorded in
`benches/results/tuning/T1/DECISION.md`. Work continues with the measurable code candidates below.

**Plain-English results:** We tried to count the individual system calls, but macOS security
settings prevented both available tracing methods. We did not guess at syscall counts, so this
story is still blocked until a privileged trace or user-supplied capture is available.

### Story T1.2 — Remove the known per-file waste
- [x] Three candidates identified by inspection: a 64 KiB buffer allocated per file
  regardless of size (`READ_BUFFER_BYTES` in `crates/xsync-core/src/source.rs`), a per-file
  clone attempt that must fail before fallback (`clone::try_clone_file` in
  `crates/xsync-core/src/local.rs`), and a BLAKE3 hash of the relative path computed per
  file for the deterministic temp name (`Sink::temporary_path`).

**AC**
- Read buffer sized `min(file_size, READ_BUFFER_BYTES)`; measured effect reported separately.
- Clone attempted only above an empirically determined size threshold; the threshold and the
  measurement that chose it are recorded.
- Each change is measured independently so the attribution is honest — no bundled "we made
  it faster".
- All 124 existing tests pass and `cargo clippy --all-targets -- -D warnings` is clean.

**Progress:** The source reader now sizes its buffer to `min(file_size, 64 KiB)` with regression
coverage. A five-repetition `congress-10k` run is checked in under
`benches/results/tuning/T1/buffer-sized/`; all 10 destination oracles passed. An independent
five-repetition APFS clone spike measured clone/copy crossover at roughly 12 MiB (0.502x at 4 MiB,
0.863x at 8 MiB, 1.130x at 12 MiB, and 1.448x at 16 MiB), so file clone attempts are now skipped
below `FILE_CLONE_MIN_BYTES = 12 MiB`.

The temporary-path hash candidate is now implemented as a shared per-transfer cache in
`Sink`, preserving the existing `.xsync.tmp.<hash>` naming contract. The independent five-
repetition comparison is recorded in `benches/results/tuning/T1/hash-baseline/` and
`benches/results/tuning/T1/hash-cached/`:

| variant | xsync median wall | xsync median CPU | paired ratio vs rsync-a | oracle |
|---|---:|---:|---:|---|
| uncached hash | 2.993 s | 10.013 s | 1.077x | 5/5 pass |
| cached hash | 3.070 s | 10.251 s | 1.020x | 5/5 pass |

The measured difference is within the run's noise policy and does not show a useful speedup.
The buffer sizing and clone-threshold changes remain the useful T1.2 changes; the hash cache is
retained for correctness-preserving repeated-path reuse, but is not claimed as a performance win.

**Plain-English results:** We addressed all three known sources of per-file overhead and tested
them separately. Smaller files now use smaller read buffers, small files skip an unhelpful clone
attempt, and repeated temporary-name lookups reuse their path hash. The last change did not make
the transfer faster in this five-run test, but every destination still matched the source.

### Story T1.3 — Hit the syscall budget target
- [~] Reduce total system time on `congress-10k` to within 1.5x of `rsync -a`.

**AC**
- `congress-10k` initial copy: system time ≤ 1.5x rsync's, wall-clock paired ratio ≥ 0.9,
  five repetitions, MAD/median ≤ 15%.
- `congress-100k` confirms the result holds an order of magnitude up, ratio ≥ 0.9.
- No regression on `manga` (large-file) or on the `congress-10k` no-op case, which is
  currently 1.91x ahead.
- Correctness oracle passes on every repetition of every cell.

**Blocker:** The current measured `congress-10k` result is 0.471x paired wall speed for xsync
against rsync-a and does not meet the 0.9 wall-ratio or 1.5x system-time targets. The 100k
confirmation is intentionally deferred until the 10k prerequisite passes. This is an engineering
gap, not a request for user action; the evidence and exact failed target are in
`benches/results/tuning/T1/DECISION.md`.

**Plain-English results:** The complete T1 performance target is not met yet. The current measured
10k run is still slower than rsync and uses too much system CPU, so the larger 100k confirmation
would not be a valid pass. This is an engineering blocker, and work should continue with the next
optimization rather than treating T1 as complete.

## Summary before T2

Lower CPU usage is not automatically the same as a faster transfer. CPU efficiency matters when
CPU or filesystem syscall work is on the critical path, or when excess work limits concurrency,
thermal headroom, or scalability. If the disk or network is already saturated, saving CPU may not
change wall time; an optimization can even be slower if it adds synchronization or bookkeeping.

For T1, the concern is still relevant because this corpus has many small files and APFS cloning can
eliminate most byte movement. In that situation, per-file filesystem work can directly affect wall
time. The primary goals should remain wall time, throughput, and correctness; the 1.5x system-CPU
limit is a guardrail for scalability rather than a goal to pursue in isolation. If current xsync
meets the wall-time target without regressions on no-op, large-file, or network workloads, further
CPU reduction should be justified by a practical performance benefit.

T1.2's buffer sizing and clone threshold are implemented and tested. The temporary-path hash cache
showed no useful speedup and should be removed before finalizing T1. T1.1 remains blocked because
macOS SIP produced empty `dtruss` tables even in the user-run privileged capture. T1.3 needs a
fresh current-binary 10k measurement before deciding whether the CPU target warrants more work.

---

## Epic T2 — Sparse-aware transfer *(deferred)*

**Decision:** Sparse-file support is not a current priority. Defer T2 until a sparse corpus or
product requirement makes it worth the implementation and filesystem-specific verification cost.
The stories and acceptance criteria below remain preserved for future scheduling; no T2 result
should be presented as measured or complete in the meantime.

`Docker.raw`: 3,721.9 GB apparent, 130.2 GB allocated (3.50%), 28.6x sparseness, 17,145
extents from 4 KiB to 5.62 GB. **The complete extent map walks in 1.02 s.** Today xsync
reads and writes all 3.7 TB, which fits on no available destination.

### Story T2.1 — Extent enumeration
- [ ] Enumerate allocated extents via `SEEK_HOLE`/`SEEK_DATA`, with a documented Windows
  path (`FSCTL_QUERY_ALLOCATED_RANGES`).

**AC**
- Enumerating `docker-raw` reproduces 130.2 GB allocated across ~17,145 extents in under
  two seconds.
- Handles both a single 5.62 GB extent and thousands of 4 KiB extents without degenerating
  into per-block I/O.
- A filesystem that does not report holes falls back to dense transfer with a recorded
  warning — never silent truncation.
- Unit tests cover: fully sparse file, fully dense file, hole at start, hole at end,
  alternating single-block extents, and a file with no holes at all.

### Story T2.2 — Transfer and reproduce holes
- [ ] Send only data extents; recreate holes at the destination by seeking rather than
  writing zeros.

**AC**
- `docker-raw` transfers ~130 GB rather than 3.7 TB, and completes on a destination where
  it currently cannot.
- Destination **allocated** size within 5% of source, verified on ext4 (`mars`) or APFS
  cross-volume. **Not on `freya`** — ZFS `compression=zstd` produces a false pass by
  discarding written zeros. This constraint is stated in the story's report.
- Content verification covers the data extents; holes are asserted to be holes, not merely
  to read as zeros.
- Corpora A–C show no regression: they have no holes and must take the dense path at
  unchanged cost.

### Story T2.3 — Baseline comparison and reporting
- [ ] Record the paired comparison honestly given that no dense baseline can complete.

**AC**
- `rsync -aS` is the paired baseline at ~130 GB; the ratio against sparse xsync is the
  gate-able number.
- `rsync -a` and pre-fix xsync are recorded as **failed rows with their ENOSPC errors**,
  never as timings, and never as a ratio.
- The "cannot complete → completes in N s" result is reported as a capability change, and
  the report states explicitly that it is not a paired speedup.
- Throughput is computed on allocated bytes per T0.5.

---

## Epic T3 — Clone at the highest unchanged subtree

Per-file cloning does not rescue per-file overhead: `congress-10k` already reports
`0 physical` bytes and is still 1.92x slower than rsync, whereas a single 206 MB file, where
one clone covers everything, is 5.0x *faster*. f2 measured per-file `COPYFILE_CLONE` at
2.70x against 22x for a tree-level `clonefile` of identical bytes.

### Story T3.1 — Subtree clone selection
- [x] Identify maximal subtrees that are wholly unchanged or wholly absent at the
  destination and clone at that root instead of decomposing to files.

**AC**
- `congress-100k` with a single changed subtree completes in time proportional to the
  changed subtree, not the whole tree.
- The crossover subtree size below which per-file work wins is measured and documented.
- Falls back correctly on cross-volume and non-reflink filesystems.
- Correctness oracle passes, including directory mtimes — the local engine has regressed
  here before.

**Progress:** The local engine now selects maximal newly-absent directory subtrees during an
incremental sync. It clones those subtrees through the existing staged clone verifier, removes
their descendant file/directory actions from the normal plan, and leaves partially-present or
changed subtrees on the ordinary per-entry path. A focused regression test covers an existing
subtree beside an absent subtree, and the existing clone fallback tests remain green.

The real-corpus smoke used `congress-10k` with `bills/hconres` intentionally absent from an
otherwise populated destination. xsync emitted one `directory-clone` event for that subtree;
the independent full oracle passed all 22,568 items with zero mismatches. A 100k-scale run then
used the same missing `bills/hconres` setup: xsync emitted one directory-clone event, transferred
675 files / 4,062,586 logical bytes in 2.55 seconds, and the independent full oracle passed all
135,466 items with zero mismatches. Evidence is recorded in
`benches/results/tuning/T3/DECISION.md`.

**Results:** The `congress-100k` source is now present and pinned by the independently captured
manifest digest `2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794` (109,615
files, 135,466 manifest items, 583,940,018 logical bytes). The rotated same-host measurements
now bracket the crossover between 4.06 MB and 7.33 MB. Timestamped phase timings and the ext4
fallback result are recorded in `benches/results/tuning/T3/CROSSOVER.md`. The bracket is scoped
to this host and is not a universal speedup claim.

The same 100k missing-subtree run on `/Volumes/XSYNC_BENCH` completed in 2.62 seconds and
passed the full 135,466-item oracle with zero mismatches. Both source and destination are APFS,
and the directory clone still succeeded across volumes, so this is cross-volume correctness
evidence but not yet the required non-reflink fallback measurement.

The ext4 receiver at `192.168.1.119` then ran the same seeded 100k case. Native xsync transferred
675 files in 2.06 seconds with `directory_clones=0` and `byte_copies=675`; the remote full oracle
passed all 135,466 items with zero mismatches. This confirms the non-reflink fallback path.

For the 4,062,586-byte `bills/hconres` subtree, five same-host APFS repetitions with the
benchmark-only `--no-directory-clone` switch gave a clone median of 2.04 seconds (MAD 0.02 s)
and a per-file median of 2.20 seconds (MAD 0.13 s). Both final destinations passed the full
oracle. This is one measured subtree-size point, not a complete crossover table.

A rotated three-repetition block then bracketed the crossover on this APFS host: cloning won at
4,062,586 bytes (2.06 s vs 2.22 s per-file), while per-file copying won at 7,328,529 bytes
(2.40 s clone vs 2.12 s per-file). Per-file copying also won at 17,729,567, 115,680,459, and
284,933,886 logical bytes. Every cell reported zero failed entries, and the final full oracle
passed the 135,466-item destination.

**Plain-English results:** xsync can now recognize a missing whole directory inside an existing
destination and copy that directory in one clone operation. On this Mac, cloning won for a
roughly 4 MB missing directory, while ordinary per-file copying won from roughly 7 MB upward in
the tested ladder. That is a local bracket, not a universal filesystem rule.

---

## Epic T4 — Persistent index and change journal

The categorical win. rsync rebuilds the entire file list every run — O(tree) regardless of
how much changed. f2 measured a warm index at 25.2 ms against 300.3 ms for
`readdir`+`fstatat` (11.9x), and found real daily churn to be a few hundred authored files
rather than the ~891k a naive count suggested.

### Story T4.1 — Index prototype and cost model
- [~] Build a read-only index over `congress-1m` and measure it. Do not integrate yet.

**AC**
- Recorded: initial build cost, steady-state memory, incremental update latency, and
  time-to-first-plan against a cold `xsync` walk of 1,318,771 entries.
- Plan for `congress-1m` with <1% changed is produced in under one second.
- Memory stays inside a documented budget at 1.3M entries.

**Progress:** The repository already contains a read-only, budgeted destination index and an
external-sort source spool; neither is integrated as a persistent sync index. A lower-scale worker
sample on the available `congress-10k` corpus measured 22,567 entries, 0.209 s destination-index
build time, 0.141 s source scan time, and 0.019 s planner time. The prototype benchmark artifacts
are in `benches/results/tuning/T4.1/`.

**Blocker:** The pinned `congress-1m` corpus is not present, so the required 1,318,771-entry build,
under-one-second sparse-plan measurement, and memory-budget result cannot be established. The
five-repetition isolated benchmark also cannot publish its report on this macOS host because
`/usr/bin/time -l` exits after `sysctl kern.clockrate: Operation not permitted`; the worker itself
does emit scan/index/planner timings, but peak RSS is not available from that failed wrapper.

**Plain-English results:** The project has a bounded read-only index prototype, and it works on the
small corpus available here. We cannot yet prove that it fits the required memory or speed budget
at 1.3 million entries because that corpus is missing and the local peak-memory measurement tool
is denied by macOS.

### Story T4.2 — Change-feed correctness under loss
- [~] FSEvents is lossy under load and *says so*. f2 observed `MustScanSubDirs` and
  `UserDropped` raised 16 times during a 40,000-file burst. A client that ignores those
  flags goes permanently stale — which presents as a correctness bug, not a performance one.

**AC**
- A burst test (`cb7` build, or 40k file creations) provably raises drop flags in the test
  environment, and the index responds by rescanning the affected subtree.
- After the burst, the index matches a full cold walk exactly. This is the acceptance test;
  latency is secondary.
- The equivalent loss path is identified and handled for Linux `fanotify` and the Windows
  USN journal, or explicitly deferred with the gap named.

**Blocker:** The current checkout has no persistent index owner, filesystem-event watcher, or
platform event dependency, so there is no existing consumer to make loss-safe. The required
FSEvents drop-flag burst also needs `cb7` or a dedicated 40,000-file fixture, neither of which is
available in the current corpus set. Implementing a watcher before T4.1's index contract exists
would not prove correctness. Linux `fanotify` and Windows USN handling are likewise not yet
implemented; their gaps must be named when the index work resumes.

**Plain-English results:** We cannot safely add the “rescan when events are dropped” behavior yet
because the index that would consume those events does not exist. The story is recorded as blocked
until the index contract and a burst-test corpus are available.

---

## Epic T5 — Dedup measurement

A measurement spike before any implementation. `cb7` holds 11.7 GB in files over 50 MB with
~4.0 GB redundant by size collision, and `debug/libreader_lib.rlib` and
`debug/deps/libreader_lib.rlib` are confirmed byte-identical at 165 MB each.

### Story T5.1 — Quantify available dedup
- [~] Run FastCDC over `cb7`'s `target/` and measure unique versus total chunk bytes.

**AC**
- Unique-byte fraction reported for a first sync and across two consecutive builds.
- **Decision gate:** below 70% unique on first sync and below 20% across two builds
  justifies implementation. Above those, dedup stays deferred and the report says so.
- Chunk-size sensitivity is swept, not assumed.
- No transfer implementation is written under this story.

**Blocker:** The `cb7` target tree is present at roughly 39 GB and 165,599 files, but this checkout
does not contain two pinned consecutive-build snapshots. One mutable target tree cannot prove the
required across-build dedup result. There is also no checked-in FastCDC measurement command yet;
the cached `fastcdc` crate is available locally, but introducing a one-off implementation without
the two-build input would still produce incomplete evidence.

**Plain-English results:** We found the build corpus needed for the first measurement, but not the
before-and-after build states needed to answer the real dedup question. No dedup percentage is
claimed until both snapshots and the chunk-size sweep are available.

---

## Epic T6 — Compression policy on real data

Measured compressibility: congress **8.6x**, manga **1.00x** (output 473 bytes larger than
input), cb7 `.o` **7.8x**, cb7 JS/JSON **4.3x**. The assumption that build artifacts are
incompressible was wrong, which also means an extension-based heuristic would have been
wrong; sampling is the correct approach.

### Story T6.1 — Per-file sampling accuracy
- [~] Measure the sample-and-skip heuristic against real mixed trees rather than uniform
  synthetic corpora.

**AC**
- False-positive rate (compressing incompressible data) and false-negative rate (skipping
  compressible data) reported for all four corpora.
- Sampling overhead under 2% of wall time on `manga`, the decisive negative control.
- At least 90% of achievable compression captured on `congress` and `cb7`.
- No per-file decision worse than the whole-corpus optimum by more than 5%.

**Blocker:** Real congress, Manga, and cb7 roots are present, but the available compression probe
does not only sample: after making each per-file decision it rereads every file in full to compute
simulated wire bytes. A three-corpus run would reread the roughly 39 GB cb7 target three times and
was stopped before producing a report. The checked-in Story 0.5 evidence uses synthetic corpus
classes, so it cannot satisfy this story's real-corpus acceptance criteria. `docker-raw` is also
empty in this checkout.

**Plain-English results:** The sampler itself has prior synthetic evidence, but the real-corpus
accuracy test is not complete. The existing probe is too expensive as written for cb7 because it
reads every file again for each sample size; it needs a bounded-wire accounting mode or a smaller,
explicitly approved real corpus before T6.1 can be claimed.

---

## Epic T7 — Parallelism shape

f2 measured eight threads of `renameat` moving 13k/s to 14k/s because APFS serializes
directory metadata mutation, while parallel copying was worth 2.43x on the same machine.
xsync currently applies one uniform worker pool to both.

### Story T7.1 — Worker sweep and policy
- [~] Sweep worker count 1–16 separately for metadata and data phases, on APFS and ext4.

**Progress:** The existing strategy benchmark exercises synthetic dispatch at 1, 2, 4, 8,
and 16 workers. The local engine currently has one internal `local_workers` setting, and the
CLI exposes no local-worker or metadata-worker control; metadata operations are not separated
from the data worker pool. The shipped benchmark therefore cannot produce the requested
metadata-versus-data filesystem sweep.

**Blocker:** The pinned `congress-100k` corpus is now available, but the acceptance test still
requires an ext4 run and separate metadata/data phase controls. The CLI exposes no local-worker
or metadata-worker control, and metadata operations are not separated from the data worker pool.
Without those controls and the ext4 run, choosing defaults from synthetic dispatch numbers would
not establish the required APFS/ext4 policy or the no-regression claims.

**Plain-English results:** We have evidence about how the in-memory work queue behaves with
different worker counts, but not about how many filesystem workers are best. The program needs
separate knobs for metadata and data work, plus repeatable APFS and ext4 test fixtures, before a
trustworthy worker policy can be selected.

**AC**
- Requires T0.4 phase timing.
- A documented policy — expected to be low fixed metadata concurrency with data concurrency
  scaled to device queue depth — beats the current uniform pool on `congress-100k`.
- No regression on `manga` or `cb7`.
- The chosen defaults link to the report that supports them.

---

## Epic T8 — Map the regimes

### Story T8.1 — Publish the crossover table
- [~] Establish the link speed below which compression and dedup dominate and above which
  syscall cost dominates.

**Progress:** Story 8.1 contains real compressible and incompressible wire-byte comparisons,
and T0.6 now has opt-in cold-cache support plus bandwidth-shaping code for SSH. However, T0.6
has no retained shaped-transfer report, so the available results do not locate a link-speed
crossover. The existing reports also contain historical multipliers outside a single published
regime table, so the documentation gate has not been audited or satisfied.

**Blocker:** A defensible T8.1 result requires actual 50, 100, and 1000 Mbit runs (with recorded
latency and cache state), using the congress and manga bounds, plus a documentation pass that
qualifies every performance multiplier by corpus, route, and regime. Those measurements need
the intended network receiver and shaping privileges; they were not run in this checkout.

**Plain-English results:** We know compression can dramatically reduce bytes for compressible
data and does little for incompressible data, but we do not yet know the network speed at which
that byte saving outweighs extra CPU work. The missing bandwidth-shaped measurements prevent a
trustworthy crossover table, so no universal speed claim should be made yet.

**AC**
- Requires T0.6. `congress` (compressible, many files) and `manga` (incompressible, large
  files) bound the answer.
- A published table names the crossover, and every subsequent performance claim in
  README/docs cites which regime it belongs to.
- No unqualified multiplier appears anywhere in the resulting documentation.

---

## Execution order

T0 first — nothing else is measurable without it. Then **T1** (largest return, unblocks
every local claim), then **T2** (cheap given a 1.02 s extent map, and closes a data-loss
hazard). T3 and T4 follow, T4 being the only item that changes the asymptotics. T5 is a
gate: it may conclude "do not build this". T6, T7, T8 are refinement and reporting.

Explicitly out of scope: further multi-stream tuning. f2 measured framing at 20–80x against
a per-file protocol and parallel streams at only 1.0–1.6x on top, and Story 8.1 reproduced
the framing half independently — batching took `deep-small` over SSH from 8.731 s to
0.343 s. Stream count is a tunable, not an architecture concern.
