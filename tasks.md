# xsync v1 — Epics, Stories & Acceptance Criteria

Companion to [plan.md](plan.md). Ordered roughly by implementation sequence; stories within an epic
are independent unless noted. "AC" = acceptance criteria.

Performance work is tuned against the real-world corpora and research spikes in
[TUNING.md](TUNING.md), with its executable work breakdown in
[TUNING-TASKS.md](TUNING-TASKS.md). Both are v2 scope. Shipping a signed, packaged binary for
Windows, Linux, and macOS is planned in [DEPLOYMENT.md](DEPLOYMENT.md), which runs in parallel. The synthetic Epic 0 corpus classes are now
legacy for performance purposes and remain the correctness fixtures.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

## Release-readiness checklist before Epic 8.1

- [~] Story 7.1 terminal progress renderer: monotonic aggregate status, scanning state,
  periodic non-TTY output, quiet/error behavior, JSONL preservation, and concise final summary.
- [~] Story 4.5 robustness follow-ups: native codec is implemented and now validated against
  Freya’s GNU rsync 3.5.0/protocol 32 receiver; real permission/disk-full/interruption,
  malformed-frame, and receiver-crash cleanup tests remain.
- [x] Benchmark observability audit: native push, pull, and multi-stream paths accumulate
  application wire bytes; benchmark records retain compression mode/level, stream count, and
  transport fields. Release-matrix execution remains Story 8.1.
- [~] Release-candidate freeze: release build and strict local gates pass, while pinned-corpus
  matrix execution and host/tool checks are still pending.
- [x] Reconcile stale task status for the prerequisite stories as evidence is collected; Stories
  8.2 and 8.3 remain downstream of the benchmark results.

---

## Roadmap corrections from `f2`

These are backlog invariants, not completed claims:

- Replace the old universal “5–10x faster than rsync” premise with comparable paired measurements.
- Treat 32 MiB batches and 16 MiB chunks as unvalidated logical-work thresholds, not wire limits.
- Separate local I/O worker count from SSH data-stream count.
- Implement and benchmark the complete one-stream protocol before choosing a multi-stream default.
- Distinguish safe restart from durable chunk-level resume.
- Preserve non-UTF-8 Unix paths before the protocol representation is frozen.
- When xsync is unavailable on a remote receiver and cannot be installed, fall back to a native
  client implementation of a supported rsync wire protocol—not SFTP, tar, or a required local rsync
  executable.
- Move the minimum viable benchmark harness ahead of the remote protocol and retain Epic 8 as the
  release benchmark/documentation pass.
- Retire the synthetic corpora as the basis for performance claims. They are the wrong shape
  (synthetic flat corpora enumerate ~10x faster than real trees, which are directory-open bound),
  the wrong scale (smoke tier is 513 items / 1.77 MB), and they hid a 5.9x CPU overhead that is
  unmissable at real scale. They remain the correctness fixtures. Tune against the three real
  corpora in [TUNING.md](TUNING.md): `congress` (1.32M files, 14 GB, 8.6x compressible),
  `manga` (117 files, 27 GB, genuinely incompressible), and `cb7` (205k files, 42 GB, mixed sizes
  with ~4 GB of duplicated build artifacts).

---

## Epic 0 — Evidence harness & architecture gates

This epic is numbered zero because it constrains unfinished v1 work. It does not invalidate the
already completed scaffold, scanner, planner, or sink stories.

### Story 0.1 — Reproducible report format and correctness oracle
- [x] Add a `benches/` harness that emits versioned JSON plus Markdown and verifies every produced
  destination with an implementation-independent manifest.

**AC**
- Every report records source revision, release build identity, hardware, OS/kernel, filesystem,
  transport/route, tool versions, stream count, compression policy, corpus manifest digest, and
  every repetition.
- Every gated result has at least five repetitions and reports median, MAD, item count, logical
  bytes, wire bytes, CPU time, peak RSS, and phase timings.
- A performance comparison occurs only for identical environment and content-pinned corpus identity;
  otherwise it is explicitly skipped. MAD/median above 15% is unverified, not passed.
- Gates compare paired ratios against a same-run baseline; method order rotates or randomizes so a
  candidate is not always measured after the baseline.
- A destination content, type, symlink-target, mode, and mtime mismatch always fails regardless of
  timing. A gate that compared nothing fails in strict/CI mode.
- First-pass and warm-cache runs are labeled accurately; the harness never calls the first
  repetition a cold-cache run without actually evicting caches.

**Results:**
- Added the standalone `xsync-bench` workspace package and CLI for independent manifest creation
  and verification, validated report generation (versioned JSON plus Markdown), rotated method
  schedules, correctness-first historical gates, and marker-owned scratch allocation/cleanup.
- The raw report schema records the revision/release build, complete machine and route identity,
  tool versions, stream/compression settings, a content-pinned corpus, and every repetition. The
  validated report derives medians, MAD, same-repetition paired ratios, counts, byte totals, CPU,
  peak RSS, cache state, and per-phase timings without discarding raw observations.
- The independent BLAKE3 manifest pins reversible native path bytes, entry type, file length and
  content, Unix mode, nanosecond mtime, and raw symlink target. Correctness failures prevent both
  current and historical results from participating in a gate.
- Comparisons require identical report schema, environment, session configuration, and corpus
  digest; noisy results are skipped as unverified, and strict mode fails if no paired comparison is
  performed. Report validation rejects fewer than five repetitions, fixed candidate ordering,
  false cold-cache labels, inconsistent counts, and failed or mismatched oracle results.
- Verification: 58 routine workspace tests pass, the existing 100k-entry scanner stress test is
  intentionally opt-in, workspace Clippy passes with warnings denied, and the release CLI smoke
  test lists all harness commands.

### Story 0.2 — Deterministic corpus and workload matrix
- [x] Generate content-pinned corpora for flat-small, deep-small, zero-byte storm, mixed,
  compressible, incompressible, and one-large-file workloads.

**AC**
- Flat and deeply nested corpora each contain 100,000 entries but have different deterministic
  topology; the manifest pins path bytes, kind, length, content digest, mode, mtime, and link target.
- Large-file size is configurable with 10 GiB as the full tier; smoke tiers run in ordinary CI.
- Workloads cover initial copy, no-op second sync, 1% content churn, metadata-only churn, type
  replacement, delete, and interrupted/resumed transfer.
- Fixture creation uses an owned scratch root with a marker; cleanup refuses unmarked, home,
  repository, filesystem-root, and out-of-scratch paths.

**Results:**
- Added the versioned `xsync.corpus.v1` generator and `xsync-bench corpus` command. Every owned run
  contains a deterministic source, a workload-specific initial destination, independent full
  manifests for both states, and an `xsync.corpus.scenario.v1` descriptor with resolved sizes,
  seed, manifest identities, and mutation/partial-copy counts.
- Added all seven required classes. Regression/full `flat-small` and `deep-small` plans each
  contain exactly 100,000 entries below the root; the deep shape uses 100 ten-level branches.
  Canonical seed-0 materializations were generated and independently manifested at exactly 100,001
  items including the root, with distinct topology/content digests, then removed through guarded
  ownership cleanup.
- Added smoke, regression, and full sizing. The allocated one-large-file defaults are 8 MiB, 1 GiB,
  and 10 GiB respectively and remain explicitly overridable. The ordinary test suite materializes
  every default smoke class; the 10 GiB full tier is definition-tested but deliberately not created
  in routine CI.
- Added initial-copy, no-op-second-sync, one-percent content churn, metadata-only churn, type
  replacement, delete, and interrupted/resume states. Selection is deterministic from the seed;
  interrupted destinations contain a stable half of leaf entries and a named staging artifact.
- Adapted the useful fixture lessons from `f2`: schema-and-seed pinning, explicit cost tiers,
  same-seed/different-seed manifest checks, and ownership-aware cleanup. Xsync's fixtures use the
  Story 0.1 BLAKE3 oracle, reversible native paths, and xsync-specific workload states rather than
  sharing `f2` implementation code.
- Verification: 65 routine workspace tests pass, the existing 100k scanner stress test remains
  intentionally opt-in, workspace Clippy passes with warnings denied, and the release CLI exposes
  the complete corpus class/tier/workload surface.

### Story 0.3 — Scanner, planner, and memory baselines
- [x] Measure the existing scanner/planner on both corpus shapes before selecting platform-specific
  enumeration or a spill-index implementation.

**AC**
- Report scan entries/s, syscall-sensitive phase time, planner time, peak RSS, and queue high-water
  marks separately.
- A 1M-entry tier demonstrates that peak memory remains within a documented budget; if the current
  destination `HashMap` exceeds it, Story 2.2b becomes a protocol-blocking requirement.
- On macOS compare the portable walker with a timeboxed `getattrlistbulk` prototype; adopt the
  platform backend only if it materially improves the deep-tree paired ratio and preserves raw path
  bytes and metadata semantics.
- On Linux record at least ext4 plus one materially different destination filesystem before making
  a cross-platform scanner claim.

**Results:**
- Added the separate `xsync-engine-bench` release runner so the Story 0.1 oracle remains independent
  from `xsync-core`. Each of at least five isolated process repetitions records both scanner passes,
  combined syscall-sensitive time and entries/s, destination-`HashMap` construction, planner time,
  true peak RSS, plan/count correctness, and producer-observed queue high-water. JSON and Markdown
  use the versioned `xsync.engine-bench.report.v1` schema and retain every repetition.
- Instrumented the existing bounded scanner with a diagnostic queue high-water counter. All tested
  100k shapes reached or nearly reached the configured 1,024-entry bound, confirming backpressure
  without changing the scanner's entry stream or capacity guarantee.
- On the Apple M1 Max/APFS host, flat-small-100k measured 637,230 entries/s and 281,264,128 B peak
  RSS; deep-small-100k measured 907,095 entries/s and 93,437,952 B peak RSS. Planner medians were
  17.0 ms and 24.5 ms respectively.
- Timeboxed `f2`'s existing macOS `getattrlistbulk+openat` prototype on the identical deep corpus.
  It measured about 658k entries/s, only 0.73x the portable scanner's rate, and its
  `String(cString:)` names plus incomplete mode metadata fail xsync's reversible-path/metadata
  contract. The platform backend is rejected; Story 2.1b remains the prerequisite for any future
  backend experiment.
- Verified `sanjee@mars.local` rather than assuming its storage: home is ext4 and `/tmp` is tmpfs.
  Recorded both flat/deep 100k shapes on both filesystems. Ext4 measured 1.13M/2.90M entries/s and
  tmpfs 0.98M/3.00M entries/s; these are one-host observations, not cross-platform claims.
- The explicit ext4 flat-small-1M tier measured 1,232,865 entries/s, 202.0 ms destination-index
  construction, 239.1 ms planning, and 478,515,200 B (456.3 MiB) peak RSS. It passes the documented
  512 MiB budget at 89.1%, so Story 0.3's conditional rule does not make the current `HashMap` a
  protocol blocker. Story 2.2b remains scheduled because the design is still unbounded and has
  little measured headroom.
- All scan MAD/median ratios were below 15%. Authoritative reports and the decision record are in
  `benches/results/story-0.3/`. Verification: 68 routine workspace tests pass, the existing 100k
  scanner stress test remains opt-in, and strict workspace Clippy passes on both macOS and Linux.

### Story 0.4 — Local clone/reflink and streaming I/O spike
- [x] Compare ordinary verified copy with APFS `clonefile`, Linux `FICLONE`, and supported
  directory-level clone behavior; separately measure no-cache/read-ahead hints for cross-volume
  large files.

**AC**
- Capability failures fall back without changing output semantics or leaving a final partial file.
- Tests cover fresh destinations, existing destinations, trailing-slash behavior, exclusions,
  metadata preservation, subsequent source mutation (copy-on-write independence), and
  `--paranoid` readback.
- Directory-level clone is selected only when the complete requested tree can be cloned without
  violating excludes, delete semantics, or destination merge behavior.
- Platform I/O hints ship only if paired results improve and cache-pressure measurements show no
  material user-visible regression.

**Results:**
- Added a staged clone/reflink spike and a versioned paired report runner. It rotates five method
  pairs, records capability disposition and honest cache state, and accepts a timing only after the
  independent manifest oracle passes. Capability and validation failures remove the sibling stage
  and physically copy before atomic publication; `--paranoid` reads back the final name.
- APFS 1 GiB file cloning measured 95.14x faster than physical buffered copy (0.0030 s vs 0.2880 s)
  and passed all five independent verifications. This capability-gated per-file path is selected
  for later local-transfer integration.
- The staged APFS `cp -c -R` tree prototype was 0.917x and is rejected. f2's actual one-call
  `clonefile(2)` root path measured 19.56x on the identical 10k-entry mixed corpus, so native
  whole-tree clone remains selected only behind the fresh/complete/no-exclude/no-delete/no-merge
  predicate and a safe platform wrapper.
- Reconfirmed `mars.local` home as ext4. All five Linux `FICLONE` attempts were unsupported and
  fell back cleanly; the 0.993x paired result matches the physical baseline and every oracle passed.
- A separate APFS-to-APFS-volume f2 probe found 32 MiB `F_NOCACHE` + read-ahead at 2.37x
  `copyfile`, with the warmed-sentinel cache ratio passing at 0.89x. No I/O hint is selected because
  the inherited throughput probe groups rather than rotates methods; shipping still requires an
  xsync-native paired improvement and repeated cache-pressure pass.
- Tests cover fresh/existing targets, trailing-slash routing, exclusions, delete/merge gates,
  metadata, symlinks/empty directories, source mutation independence, partial and corrupt clone
  failures, fallback cleanup, and paranoid readback. Full verification passed 75 routine workspace
  tests on macOS and 76 on Linux (plus one existing
  opt-in stress test on each); strict workspace Clippy passes on both. Evidence and the complete
  decision are in `benches/results/story-0.4/`.

### Story 0.5 — Remote baseline and default-decision matrix
- [x] Measure actual `xsync` single-stream framing, then 2/4/8 streams and compression choices,
  against fair rsync baselines on real SSH hosts.

**AC**
- Baselines include `rsync -a` and a compressed rsync mode supported by both endpoints; tar is
  reported only as an archive-stream reference, never as an `xsync` proxy.
- At least two destination filesystem classes and one resource-constrained host are represented.
- Stream-count results use the same corpus/session conditions and include SSH setup time.
- Omitted `--streams` resolves to four only if it improves the paired ratio on two materially
  different filesystems without regressing the validation host by more than 10%; otherwise it
  resolves to one. CPU count is not the stream-count heuristic.
- Compression tests compare 64 KiB, 256 KiB, and 1 MiB sampling on compressible, incompressible,
  mixed, and short-file payloads; the selected default has less than 2% overhead on incompressible
  data.
- The matrix records native-xsync and rsync-protocol fallback separately; fallback setup time,
  negotiated dialect, feature map, correctness, and degraded guarantees are visible.

**Results:**
- A benchmark-only xsync framing executable measured 1/2/4/8 persistent SSH streams against
  reference `rsync -a` and `rsync -az` on real ext4 and tmpfs destinations. All five-repetition,
  order-crossed samples passed a separate remote BLAKE3 manifest; reports include setup phases,
  CPU, peak RSS, application-wire bytes, hardware/OS/kernel identity, and exact corpus identity.
- Omitted `--streams` resolves to **one**. Four improved tmpfs by a verified paired 1.116x, but its
  ext4 1.026x result had 21.4% relative paired MAD and cannot satisfy the required second-filesystem
  gate. An explicit 1-CPU/512-MiB ext4 receiver profile showed no greater-than-10% regression.
- Adaptive zstd level 3 uses a 64 KiB sample and selects at a ratio of 0.95 or lower. All three
  tested sample sizes made identical decisions across short, compressible, incompressible, and
  mixed corpora; incompressible wire overhead was exactly 0%, while a compressible native transfer
  improved 4.295x and reduced application bytes from 268,482,672 to 83,056.
- Native framing is explicitly a pre-production spike. Reference rsync negotiated protocol 32;
  native rsync-protocol fallback remains Story 4.5 and its unavailable setup/guarantees are visible
  rather than fabricated. Tar was not used as an xsync proxy. Full decision and evidence:
  `benches/results/story-0.5/DECISION.md`.

**Epic 0 current audit (2026-08-24):** Stories 0.1–0.5 are all complete. The current workspace
passes `cargo fmt --all -- --check`, strict workspace Clippy, and the full workspace test suite:
166 tests passed with one pre-existing 100k-entry scanner stress test intentionally ignored. The
historical Story 0 reports remain authoritative for their recorded hosts and runs; current source
provenance is tracked separately because the worktree contains changes beyond the recorded report
builds.

---

## Epic 0 plain-English summary

Epic 0 bootstrapped the evidence and benchmarking environment for xsync. It did not mean that the
finished product was complete. It established the repeatable tools, test data, correctness checks,
and measurements that the remaining implementation stories depend on.

In practical terms, Epic 0:

- built a benchmark harness that creates test data, measures runs, and independently verifies every
  destination;
- added deterministic fixtures for small files, deep trees, empty files, mixed data, compression,
  incompressible data, and large files;
- measured scanner memory and performance and rejected a slower/incomplete macOS-specific scanner;
- tested local copy and clone/reflink options with safe capability-based fallbacks;
- measured remote transfer behavior against real `rsync` on ext4 and tmpfs filesystems;
- selected evidence-based defaults: one remote stream and adaptive zstd compression with a 64 KiB
  sample; and
- documented what is proven, what remains experimental, and what later stories must implement,
  including the native rsync-protocol fallback.

Epic 0 is therefore the project's measurement and architecture foundation, not the final transfer
product.

---

## Epic 1 — Workspace scaffold & CLI foundation

Cargo workspace, argument parsing, and rsync-compatible path semantics. Everything else builds on this.

### Story 1.1 — Cargo workspace scaffold
- [x] Workspace with `crates/xsync-core` (library) and `crates/xsync` (binary), shared lints/profile, release profile with LTO + codegen-units=1.

**AC**
- `cargo build` and `cargo test` succeed on a clean checkout.
- `cargo run -- --help` prints usage.
- Release binary is a single static-ish executable with no runtime asset dependencies.

**Results:**
- `cargo build` and `cargo test` pass (2 core unit tests; clippy clean under shared `all`+`pedantic` lints).
- `cargo run -- --help` prints full usage, including the version banner from `xsync_core::version()`.
- Release binary is a single Mach-O arm64 executable, 571 KB (`strip`+`lto`+unit `codegen-units=1`), with no runtime asset dependencies.

### Story 1.2 — CLI argument surface
- [x] Clap-based CLI: `xsync [flags] SRC DEST` plus hidden `--server` mode; flags: `--streams N`, `--delete`, `--exclude GLOB` (repeatable), `-n/--dry-run`, `--checksum`, `--paranoid`, `--progress-json`, `--no-compress`, `--compress-level L`, `-q/--quiet`, `-e/--rsh CMD`.

**AC**
- `xsync --help` documents every flag with rsync-familiar wording.
- Unknown flags and missing SRC/DEST produce a non-zero exit and a one-line error, not a panic.
- `--streams` accepts 1–16; out-of-range values are rejected with a clear message.

**Results:**
- `xsync --help` documents all flags in rsync-familiar wording; hidden `--server` is present but not shown.
- `--bogus` → `error: unexpected argument '--bogus' found` (exit 2); missing DEST → `error: the following required arguments were not provided: <DEST>` (exit 2) — clean errors, no panic (SRC/DEST required unless `--server`).
- `--streams 99` → `error: invalid value '99' for '--streams <N>': 99 is not in 1..=16` (exit 2); `--compress-level` range 1..=22 enforced the same way.
- 4 new unit tests: full flag surface, error-kind checks (unknown arg / missing arg / value validation), `--server` path not required, and default `--streams=None`; clippy clean.

**Resolved:** Story 0.5 selected one stream, the help text records that decision, and Story 4.2 now
resolves an omitted value to one at runtime while honoring explicit values in the supported range.

### Story 1.3 — Path spec parsing (rsync conventions)
- [x] Parse `[user@]host:path` vs local paths; Windows drive letters (`C:\foo`) are not mistaken for hosts; trailing-slash semantics captured (`path/` = contents, `path` = include the directory itself).

**AC**
- Unit tests cover: `dir`, `dir/`, `host:dir`, `user@host:dir/`, `C:\Users\x` (Windows-local), `./relative`, single file source.
- `xsync a b` produces `b/a/...`; `xsync a/ b` produces `b/...` (verified in Epic 2 integration tests).
- Remote→remote (`host1:a host2:b`) is rejected with a clear "not supported in v1" error.

**Results:**
- New `xsync_core::path` module: `parse()` → `PathSpec { location (Local | Remote{user,host}), path, trailing_slash }`, `find_remote_colon()` treats single-letter+`/`or`\` as a Windows drive (never a host), and `validate_pair()` rejects remote→remote.
- 8 new unit tests in `path` (plus the 2 lib tests, 4 CLI tests): `dir`, `dir/`, `host:dir`, `user@host:dir/`, `C:\Users\x` and `C:/Users/x` (local), `./relative`, single-file source, empty/`host:` errors, and remote→remote rejection. clippy clean.
- Binary: `xsync h1:a h2:b` → `xsync: remote-to-remote sync is not supported in v1` (exit 1); local→remote and Windows-drive source parse fine (exit 0, no work performed yet).
- Trailing-slash → internal dest path prefix behavior (`b/a/...` vs `b/...`) is wired into the `PathSpec` flag; output-tree integration is exercised in Epic 2, per its AC.

### Story 1.4 — Transport selection and fallback UX
- [x] Add `--transport=auto|xsync|rsync` with `auto` as the default and model transport
  capabilities explicitly in the engine/event API.

**AC**
- `auto` tries the native xsync receiver first and considers rsync only for a positively identified
  “xsync executable unavailable” result after any separately authorized install/bootstrap path is
  unavailable or declined.
- `xsync` never falls back; `rsync` skips the xsync probe and requires a supported remote rsync
  server dialect.
- Authentication/host-key errors, native protocol mismatch/corruption, and failures after remote
  mutation begins never trigger automatic fallback.
- Selection happens before planning assumptions or destination mutation. Human and JSON output name
  the selected transport, remote implementation/version, negotiated wire version, supported
  features, and degraded guarantees.
- The CLI never uploads or installs a binary without a separate explicit user-authorized action.
- Local→local rejects `--transport=rsync` as inapplicable rather than spawning an unnecessary child.

**Results:**
- Added `--transport=auto|xsync|rsync`, defaulting to `auto`, plus a shared transport-selection
  model containing backend, remote implementation/version, wire version, mapped options,
  capabilities, unavailable guarantees, and selection reason.
- `auto` selects native xsync first and falls back only on the explicit missing-remote-xsync result;
  authentication, host-key, protocol, corruption, and post-mutation failures remain hard failures.
  Explicit `xsync` never falls back, and explicit `rsync` skips the native probe.
- Human progress output and `--progress-json` both expose the selected transport contract. The CLI
  never installs or uploads a receiver implicitly, and local-to-local `--transport=rsync` is rejected
  before any work begins.
- Native rsync-wire fallback uses the local codec and a remote reference receiver without requiring a
  local rsync executable. Unsupported guarantees and mapped options are rejected before mutation.
- Verification: the remote integration suite covers missing-native fallback, authentication and
  host-key non-fallback, protocol failure non-fallback, shell quoting, unsupported-option rejection,
  nonzero receiver exit, final transport JSON, and rsync correctness. Full workspace verification
  currently passes 166 tests with one intentional stress test ignored; strict Clippy passes.

**Known limitation (not an Epic 1 blocker):** rsync fallback is currently local-to-remote only;
remote-to-local remains explicitly unsupported until the sender dialect is implemented and tested.

---

## Epic 1 — Implementation Report

Epic 1 delivered the cargo workspace, complete CLI surface, rsync-compatible path parsing, and
transport selection/fallback UX. All four stories meet their stated AC; the only documented
limitation is rsync fallback's remote-to-local direction.

**Layout**
- Root `Cargo.toml`: workspace (`resolver 2`, members `crates/xsync`, `crates/xsync-core`), shared `[workspace.package]` (edition 2021, rust-version 1.88), `[workspace.lints]` (`unsafe_code = deny`, clippy `all`+`pedantic` warn), release profile `lto`+`codegen-units=1`+`strip`.
- `crates/xsync-core` — engine library. `version()`, `PROTOCOL_VERSION`, `HANDSHAKE_MAGIC`; new `path` module (`PathSpec`, `Location`, `parse`, `validate_pair`, `PathError`). Deps: `thiserror`.
- `crates/xsync` — binary. clap-derived `Cli` (struct); `run()` parses SRC/DEST and rejects remote→remote; returns `ExitCode` on error. Deps: `clap`, `xsync-core`.

**What works**
- `cargo build` / `cargo test` pass in the current workspace; the latest full run passed 166 tests
  with one intentional 100k-entry scanner stress test ignored, and strict Clippy is clean.
- `cargo run -- --help` documents every flag; hidden `--server` present but not shown.
- Unknown flag, missing SRC/DEST, and out-of-range `--streams`/`--compress-level` → exit 2 with a one-line clap error (no panic); remote→remote → exit 1 with `xsync: remote-to-remote sync is not supported in v1`.
- Path parsing handles `dir`, `dir/`, `host:dir`, `user@host:dir/`, `./relative`, single-file source, and Windows drive letters (`C:\Users\x`, `C:/Users/x`) — never misread as hosts.
- Release binary: single Mach-O arm64 executable (~600 KB), no runtime asset dependencies.
- `--transport=auto|xsync|rsync` selects a capability-described backend before mutation. Auto falls
  back only for a positively identified missing remote xsync executable; explicit modes never
  silently switch backends. Human and JSON output include transport, implementation, wire version,
  mapped options, and unavailable guarantees.
- The native rsync sender speaks the selected whole-file receiver dialect without requiring a local
  rsync executable. Its unsupported options and guarantees fail before remote mutation.

**Verified beyond the parser (per Story 1.3 AC)**
- Output-tree trailing-slash semantics: `xsync a b` → `b/a/...` vs `xsync a/ b` → `b/...` are
  captured in `PathSpec.trailing_slash` and verified by the local and remote integration suites.

**Interface for Epic 2**
- `PathSpec { is_remote(), host(), path, trailing_slash }` and `xsync_core::path::{parse, validate_pair}` are the entry points the local engine's scanner → planner → transfer should consume.

---

## Epic 1 plain-English summary and next steps

Epic 1 turned the project foundation into a usable command-line shape. We established the Cargo
workspace, built the `xsync` command, documented the rsync-style options, and made path handling
understand local paths, remote paths, trailing slashes, and Windows drive letters.

We also added transport selection. By default, xsync tries its native receiver first and only uses
the rsync-protocol fallback when the remote xsync executable is positively unavailable. Explicit
transport choices are respected, failures such as authentication or protocol errors do not trigger
fallback, and human-readable plus JSON output describe the selected transport and its guarantees.

The remaining limitation is intentional: rsync fallback currently supports local-to-remote
transfers. Remote-to-local fallback waits for the sender-side rsync dialect work in a later story.

Next steps are to build the local synchronization engine: scan the source, compare it with the
destination, plan the required changes, transfer files safely, verify the results, and preserve
rsync-compatible path and trailing-slash behavior. Later work will then wrap that engine in the
native remote protocol and complete the remaining fallback direction.

---

## Benchmark and tuning work in plain English

We also bootstrapped a repeatable way to measure xsync against rsync. The benchmark tools can
create or use content-pinned corpora, run the same workload several times, verify the destination
independently, and record timing, CPU, memory, wire-byte, and phase information. This gives us a
trusted baseline instead of relying on one-off timings or synthetic files that do not resemble the
real project data.

The first real-corpus tuning pass tested a smaller read buffer for small files. The change is safe
and covered by unit tests, but it did not solve the main performance gap: on the Congress corpus,
xsync's median wall time was 6.724 seconds versus 3.123 seconds for rsync. The destination was
correct in every repetition, so this is a performance finding rather than a correctness failure.

The next steps are to capture syscall-level evidence on a host where tracing is permitted, then
measure and implement the remaining per-file optimizations (including clone eligibility and hash
work) one at a time. The current macOS environment blocks `dtruss` and `fs_usage` without root,
and the target performance budget is not yet met; those are recorded as tuning blockers rather
than hidden assumptions.

## Epic 2 — Local sync engine (scan → plan → transfer → verify)

The full pipeline working local→local, in-process, with strategy switching and BLAKE3 verify-in-flight. This alone is a useful fast `cp -r` replacement and is the foundation the protocol wraps.

### Story 2.1 — Parallel scanner
- [x] Multi-threaded directory walk (`ignore::WalkParallel` with ALL standard filters disabled — dotfiles and gitignored files are included) streaming entries (rel path, kind, size, mtime, mode) into a channel; does not follow symlinks.

**AC**
- Scanning a tree with `.git/`, `.gitignore`d files, and dotfiles includes all of them (rsync parity).
- Symlinks are reported as symlinks, never traversed; a symlink loop does not hang the scan.
- Rel paths are `/`-separated on all platforms (protocol canonical form).
- Scan of 100k files completes without unbounded memory growth (streaming, bounded channel).

**Results:**
- New `xsync_core::scanner` module exposes `scan()` / `scan_with_capacity()` and streams `FileEntry { path, kind, size, mtime, mode }` from `ignore::WalkParallel`; standard filters and symlink following are explicitly disabled.
- Relative paths are assembled from platform path components with `/`; directory roots are omitted, while a single-file or symlink root is emitted under its basename.
- The default crossbeam channel is bounded to 1,024 entries and supports a caller-selected bound; tests exercise producer backpressure at capacity 2 and a 100,000-file stress scan at capacity 32.
- 7 scanner tests cover `.git`, `.gitignore` matches, dotfiles, symlink loops, canonical relative paths, bounded streaming/stress, single-file roots, and invalid capacity. Full workspace: 22 routine tests pass (the 100k stress test is opt-in and also passes); clippy is clean with warnings denied, including a Windows all-target compile.

**Known delta after the `f2` review:** the current `String` protocol path rejects non-UTF-8 Unix
names, and the 100k stress test proves bounded queueing but records no performance or total process
memory. Story 2.1b supersedes those parts before protocol freeze.

### Story 2.1b — Reversible wire paths and stable source fingerprints
- [~] Replace the string-only relative path with a reversible component representation and extend
  scanned metadata with the source identity needed for stable reads and cache validation.

**AC**
- Unix tests round-trip names containing invalid UTF-8 bytes through scan → plan → protocol codec →
  sink without lossy conversion. Windows tests round-trip nontrivial Unicode and reserved-prefix
  cases through the documented Windows encoding.
- Wire paths are relative component sequences; absolute/rooted paths, empty components, `.`, `..`,
  NUL, and platform prefixes are rejected before any filesystem mutation.
- Destination collision checks account for exact duplicates and the destination's case and Unicode
  normalization behavior; ambiguous inputs fail before transfer.
- `FileEntry` carries a platform source fingerprint including stable file identity, size, precise
  mtime, ctime/change-time where available, and kind.
- Symlink targets use the same reversible platform representation and are never followed during
  discovery.

**Progress:** Source fingerprints, raw-byte protocol fields, rsync fallback path handling, traversal
rejection, Unix invalid-byte scan → plan → protocol → sink coverage, and the coordinated `WirePath`
representation through scanner, planner, source reader, sink, journal, and native protocol are
implemented. Exact duplicate paths are rejected before transfer, and symlink discovery remains
non-following.

**Remaining:** Windows-specific reversible encoding, reserved-prefix handling, and Windows case/
Unicode-normalization collision tests require a Windows test environment. The story remains in
progress until those platform-specific acceptance criteria are verified.

### Story 2.2 — Planner / diff classification
- [x] Given source entries and a destination index, classify: new / changed (size or mtime differs) / unchanged / extraneous. Collect dirs and symlinks separately.

**AC**
- Unit tests: identical file skipped; size-change and mtime-change detected; file present only in dest is classified extraneous; type change (file→dir, file→symlink) is handled by replace.
- Unchanged files are never opened or read in the default (non-`--checksum`) mode.

**Results:**
- New `xsync_core::planner` module provides `build_destination_index()` and metadata-only `plan()` classification. Matching destination paths are removed as source entries stream through; the index remainder becomes the extraneous set.
- `Plan` separates files, directories, symlinks, and other filesystem objects, each with `new`, `changed`, `unchanged`, and `extraneous` buckets. Type mismatches enter the source kind's `changed` bucket so the sink can replace the existing object.
- Default equality uses only kind, size, and mtime. The planner has no filesystem root or file handle, structurally preventing content reads in non-`--checksum` mode.
- 6 planner tests cover identical files, size and mtime changes, destination-only entries, file-to-directory and file-to-symlink replacement, kind-separated output, and metadata-only classification of a nonexistent path.

### Story 2.2b — Bounded destination index and planning spool
- [x] Introduce a `DestinationIndex` abstraction with an in-memory implementation and an owned,
  per-run disk-backed spill implementation selected by an explicit memory budget.

**Story 0.3 evidence gate:** the current implementation peaks at 456.3 MiB for 1M flat entries on
ext4, below the provisional 512 MiB cap but using 89.1% of it. This does not trigger Story 0.3's
conditional protocol-blocking rule; the story remains required by the architecture because the
current two scan vectors plus `HashMap` have no corpus-independent bound.

**AC**
- The same planner conformance suite runs against both implementations and produces identical,
  deterministic classifications.
- A 1M-entry destination completes under the Story 0.3 RSS budget without retaining an unbounded
  source vector while the destination scan finishes.
- The per-run store has a schema/version marker, is never reused as filesystem truth, and is removed
  after success; stale stores are safely ignored and recoverably cleaned.
- Destination scan failure prevents transfer. Source discovery may spool concurrently, but ordinary
  update transfers do not begin until exact destination classification is possible.
- Delete candidates are not executed until source discovery, transfer, and verification all finish
  successfully.

**Results:**

- `xsync_core::planner::DestinationIndex` now uses a sorted in-memory index below its explicit
  budget and spills to a marker-owned per-run store above it. Disk runs are externally sorted and
  merged with bounded fan-in, so the index never retains a path-to-entry map after spilling.
- `PlanningSpool` appends source metadata to the same versioned record format and returns a sorted
  streaming cursor. `try_plan_spooled()` and `classify_stream()` classify only after source and
  destination discovery are complete, with deterministic output and duplicate-path rejection.
- Run stores preserve precise and pre-epoch mtimes, reject malformed records and oversized paths,
  clean up on ownership drop, and expose marker-validated stale-store cleanup. The engine benchmark
  now feeds scans directly into the bounded index and source spool rather than retaining two scan
  vectors.
- Tests cover memory/disk conformance, forced spilling, deterministic classifications, timestamp
  round-tripping, duplicate paths, stale-store cleanup, and the existing planner suite.

### Story 2.3 — Strategy split & work dispatch
- [x] Files bucketed by size: small (<1 MiB) → coalesced batches (~32 MiB target); medium (1–32 MiB) → single-message whole file; huge (>32 MiB) → 16 MiB chunks striped across all workers. Bounded work queue feeding N workers.

**AC**
- A tree of 50k×4 KiB files is transferred as batches (observable via debug event counts), not 50k individual transfers.
- A single 1 GiB file engages every worker concurrently (chunk events from multiple worker ids).
- Memory stays bounded (~streams × 64 MiB worst case) regardless of input size.

**Results:**
- New `xsync_core::strategy` module defines metadata-only `SmallBatch`, `WholeFile`, and `Chunk` work items with exact task thresholds: <1 MiB, 1–32 MiB inclusive, and >32 MiB; batch target is 32 MiB and chunk size is 16 MiB.
- `bounded_work_queues()` creates one bounded crossbeam queue per stable worker id. `WorkDispatcher` streams input, applies backpressure, round-robins batches/whole files, and stripes each huge file's chunks across all workers.
- `DispatchStats` exposes batch, batched-file, whole-file, and chunk event counts. Batch metadata is additionally capped at 8,192 entries so empty files cannot cause unbounded accumulation; queued work contains metadata only, and payload units are capped at 32 MiB batches or 16 MiB chunks.
- 5 strategy tests cover size boundaries and chunk ranges, 50,000×4 KiB coalescing (7 batches), 1 GiB striping (64 chunks across all 8 workers), bounded queues and empty-file batch caps, and invalid input/configuration.

**Known delta after the `f2` review:** 32 MiB batches and 16 MiB chunks are hypotheses. The current
per-worker round-robin queues can block dispatch on one full queue while other workers are idle, and
logical work sizes must not become unbounded wire-frame allocations.

### Story 2.3b — Strategy calibration and shared work scheduling
- [x] Calibrate size/batch/chunk thresholds with Story 0 and separate local worker scheduling from
  transport stream assignment.

**AC**
- Small and medium local work drains from a shared bounded queue or equivalent work-stealing design;
  a deliberately slowed worker does not stall dispatch to idle workers.
- Network stream assignment remains stable where protocol ordering requires it, and no two workers
  write the same large-file range.
- Logical batches can span several protocol frames; changing the wire payload limit does not change
  which files belong to a logical batch.
- Benchmarks sweep at least 8/16/32/64 MiB batch targets, 4/8/16 MiB data segments, and worker counts
  1/2/4/8/16 on flat-small, deep-small, and large-file corpora.
- Selected constants and their evidence are recorded in a checked-in decision report. Peak buffered
  payload remains within a documented global budget even at `--streams 16`.

**Results:**

- `StrategyConfig` makes logical small-file, whole-file, batch, and chunk thresholds explicit and
  independent from wire-frame limits. The checked-in calibration runner sweeps 8/16/32/64 MiB
  batches, 4/8/16 MiB chunks, and 1/2/4/8/16 workers across flat-small, deep-small, and large-file
  synthetic corpora with five repetitions per cell.
- `shared_bounded_work_queues()` gives local workers cloned access to one bounded queue while
  large-file ranges remain on stable per-stream queues. A slow local worker therefore cannot block
  idle workers, and range ownership remains deterministic and disjoint.
- Logical work remains metadata-only. `logical_queue_bound_bytes()` records a conservative 576 MiB
  logical reservation at queue capacity 2 and 16 streams for the default thresholds; the decision
  report documents that this is not a payload allocation and that transport byte limits remain
  authoritative.
- `benches/results/story-2.3b/DECISION.md`, `strategy-matrix.json`, and `strategy-matrix.md`
  record the selected 32 MiB batch and 16 MiB chunk defaults and their matrix evidence.

### Story 2.4 — Verified write path (sink)
- [x] Writes go to `.xsync.tmp.<hash-of-relpath>` in the destination directory; BLAKE3 verified before commit; mode+mtime set on temp; atomic rename to final name. Parent dirs created on demand. Empty dirs, dir modes, symlinks recreated; dir mtimes set in a depth-first pass at the end.

**AC**
- After sync, source and dest trees are byte-identical (recursive hash compare in tests) with matching mtimes and unix permissions, including empty dirs and symlink targets.
- Kill -9 mid-transfer leaves only `.xsync.tmp.*` files — never a truncated file under its final name; re-running safely restarts the file (temp names are deterministic, so leftovers are overwritten, not accumulated).
- A file whose received bytes fail hash verification is retransmitted once; a second failure reports the file as failed and exits with the partial-failure code.

**Results:**
- New `xsync_core::sink::Sink` writes complete files through deterministic `.xsync.tmp.<BLAKE3-relpath>` names, verifies expected length and BLAKE3, applies mode+mtime to the temp, then renames it into place. Protocol paths are validated against absolute/traversal escapes.
- Whole-file and chunk APIs retransmit once on corruption. A second mismatch returns typed `SinkError::VerificationFailed` without replacing an existing destination, ready for Story 2.5's partial-failure exit mapping.
- Chunked files support prepare/preallocate, independently verified disjoint range writes, and final metadata+commit. Empty directories are created explicitly, parents on demand, symlinks via deterministic temps, and directory metadata is restored deepest-first after child writes.
- 6 sink tests cover corruption recovery, repeated verification failure, deterministic interrupted-transfer restart safety, disjoint chunk assembly, unsafe paths, and complete tree fidelity (bytes, mtimes, Unix permissions, empty directories, and symlink targets).

**Scope correction:** Story 2.4 proves atomic publication and safe restart. It does not prove durable
chunk-level resume across process loss; that is Story 3.4.

### Story 2.4b — Stable source read contract
- [x] Add a source reader that detects replacement or mutation between scan, open, read, and final
  verification rather than hashing a potentially mixed source version.

**AC**
- The reader opens without following symlinks, compares the opened descriptor with the scanned
  fingerprint before reading, and compares descriptor plus pathname state after reading.
- A first change triggers a fresh scan and one retry; a second change becomes a named partial
  failure. A vanished file is reported without aborting unrelated work.
- Tests replace the pathname during reading, truncate/extend in place, rewrite while preserving
  length, and swap a regular file for a symlink. No test publishes mixed-version bytes.
- Hash-cache population and remote resume checkpoints occur only for a stable completed read.

**Results:**

- `scanner::FileEntry` now carries a `SourceFingerprint` with kind, size, precise mtime, ctime where
  available, and platform file identity. The planner spool schema was bumped and preserves the
  fingerprint fields.
- New `xsync_core::source::SourceReader` opens regular files without following Unix symlinks,
  compares descriptor and pathname fingerprints before/after the read, hashes while reading, and
  retries one freshly fingerprinted version after a race. A second change returns named
  `SourceReadError::Unstable`; disappearance returns `SourceReadError::Vanished`.
- Tests cover pathname replacement, in-place truncation and extension, same-length rewrites,
  regular-file-to-symlink replacement, vanished files, stable BLAKE3 output, and no mixed-version
  publication. `StableRead` is the only successful read result, so downstream hash-cache or resume
  consumers receive data only after final stability verification.

### Story 2.5 — Local→local end-to-end
- [x] `xsync src/ dest` with both sides local runs source+sink directly in-process (no child
  protocol serialization); local I/O workers are configured independently from SSH streams.

**AC**
- Integration test: generated tree (nested dirs, small files, one 100 MiB file, symlinks, empty dirs) syncs correctly; second run transfers 0 bytes and reports all files skipped.
- Vanished and changed-source races use Story 2.4b and produce warnings/partial-failure status without
  aborting unrelated files.
- Trailing-slash semantics are verified from the observable output tree for file roots and directory
  roots.
- Local worker count is reported in events and does not change when `--streams` is supplied for an
  all-local transfer.

**Results:**

- `xsync_core::local::sync` provides the direct local route: bounded shared local file work is
  consumed by independently configured I/O workers, source payloads go through `SourceReader`, and
  verified bytes are published by `Sink` without protocol or child-process serialization.
- `LocalEvent` and `LocalSyncReport` expose planned bytes, transferred/skipped files, warnings,
  failures, deletes, local worker count, and the requested stream count. The CLI renders these as
  terminal events or JSONL; local `--streams` is observable but does not change local worker count.
- Source and destination root layout preserves file roots, directory roots, and trailing-slash
  contents semantics. Directory metadata is applied after child writes, while deletes are delayed
  until all transfer work succeeds. Per-entry source races and sink failures become warnings and
  partial status (`23`) while unrelated queued files continue.
- Tests cover nested directories, an empty directory, symlink targets, a sparse 100 MiB file,
  recursive metadata publication, second-run zero-byte transfer with file skip events, file-root
  placement, trailing-slash placement, safe delayed deletion, and worker/stream reporting.

### Story 2.6 — Local clone/reflink fast path
- [x] Implement the fast paths selected by Story 0.4: highest valid directory clone, per-file
  reflink/clone, then verified streaming fallback.

**AC**
- APFS and Linux reflink-capable tests demonstrate capability detection, correct fallback errors,
  atomic publication, copy-on-write independence, and identical logical output.
- A fresh complete destination may use a directory clone; destination merges, excludes, or otherwise
  incompatible semantics automatically decompose without changing results.
- Events distinguish directory clone, file reflink, and byte-copy work and report bytes logically
  copied separately from bytes physically transferred.
- `--paranoid` re-reads cloned output; normal mode does not erase the clone benefit by reading all
  bytes solely to reproduce the streaming hash path.

**Results:**

- `xsync_core::clone` now stages platform clone attempts through APFS `cp -c` or Linux
  `--reflink=always`, validates source fingerprints and staged objects, and atomically publishes
  only complete results. Unsupported or invalid clone attempts remove their stages and return to
  the existing stable verified byte-copy path.
- Local routing selects a whole-directory clone only for an absent, complete destination with no
  exclusions or delete semantics. Existing merges, exclusions, and delete requests decompose into
  per-file clone attempts followed by verified streaming fallback. Exclude globs are applied to
  both source and destination planning paths.
- `TransferMethod`, local events, and reports distinguish directory clones, file clones, and byte
  copies. Logical bytes and streaming physical bytes are reported separately. `--paranoid` hashes
  cloned stage/final output; normal clone mode performs metadata/fingerprint validation without
  re-reading payloads solely for the streaming hash path.
- Tests cover atomic stages, absent-target eligibility, clone capability fallback, copy-on-write
  independence, paranoid tree/file verification, exclusion decomposition, and complete workspace
  behavior. The existing APFS/Linux clone spike remains the platform-specific benchmark oracle.

## Epic 2 status in plain English

Epic 2 now has a working local synchronization engine: it scans trees, compares source and
destination state, plans bounded work, reads changing files safely, writes through verified temporary
files, publishes atomically, preserves metadata and symlinks, and reports partial failures without
losing unrelated work. It also supports batching, large-file striping, local clone/reflink fast
paths, and a complete local-to-local command path.

The remaining blocker is Story 2.1b. Native xsync still needs a reversible raw path representation
through its scanner and planner before invalid Unix filenames and platform-specific Windows path
cases can be guaranteed end-to-end. All other Epic 2 stories are complete and the existing tests
continue to pass; implementation can proceed to the next epic while Story 2.1b is resolved before
the protocol representation is frozen.

---

## Epic 3 — Protocol, server mode & PipeTransport

The wire protocol and `xsync --server`, exercised over child-process stdio — byte-identical to the SSH path without needing sshd.

### Story 3.1 — Framing & message types
- [x] Specify and implement a versioned fixed envelope plus bounded typed payloads. Postcard may
  implement payload encoding, but checked-in field layouts and golden bytes—not Rust enum layout—are
  the compatibility contract. Messages cover handshake/session config, bounded batch/file segments,
  large-file prepare/ranges/finish, metadata operations, scans/stats, acknowledgements, errors, and
  paged resume state.

**AC**
- `protocol.md` specifies magic, version, type, flags, byte order, field order, path encoding,
  maximum collection counts, error behavior, and compatibility rules.
- Initial limits are explicit and testable: 16 MiB complete payload, 8 MiB data segment, 1 MiB
  encoded path, 32 MiB default unacknowledged window, and bounded declared decompressed length.
  Story 0 may revise them before freeze, but protocol v1 never silently reinterprets them.
- The receiver validates header length before body allocation/read, checked arithmetic precedes
  allocation, and oversized counts, paths, compressed output, or resume pages fail cleanly.
- Exact golden-byte and round-trip tests cover every message variant. Truncation at every byte,
  trailing bytes, unknown magic/version/type/flags, bit flips, duplicate IDs, overlapping ranges,
  and compression bombs never write a wrong final file or allocate beyond the session budget.
- Logical small-file batches and logical large-file chunks may span multiple protocol frames; no
  serializer constructs a 32/64 MiB contiguous frame merely because the strategy selected that
  logical work size.
- Protocol version mismatch produces the error "xsync version mismatch: local vX / remote vY" and a non-zero exit.
- A fuzz target exercises envelope and payload decoding with a corpus of golden and malformed frames;
  its bounded CI smoke run completes without panic or excessive allocation.

**Results:**

- `protocol.md` freezes the v1 `xsn1` 32-byte little-endian envelope, type assignments, field order,
  raw path encoding, collection limits, zstd flag, version mismatch text, and fail-closed
  compatibility rules. `xsync_core::protocol` implements the checked field-level encoder/decoder.
- Payloads are capped at 16 MiB, data segments and ranges at 8 MiB, paths at 1 MiB, resume and
  metadata collections at bounded counts, and the default unacknowledged window at 32 MiB.
  Declared decompressed length is checked before zstd output allocation; encoded message size is
  checked before constructing the payload vector.
- `FrameDecoder` validates the fixed header before body allocation/read, tracks duplicate IDs within
  a bounded session budget, and rejects unknown magic/version/type/flags, truncation, trailing
  bytes, invalid booleans/enums, oversized counts, invalid UTF-8 text, and overlapping ranges.
- Tests cover every typed message round trip, handshake golden bytes, malformed-frame truncation at
  every byte, trailing bytes, version/type/flag changes, bit flips, duplicate IDs, raw non-UTF-8
  paths, resume overlap, compressed output bounds, and zstd round trips. A standalone `fuzz/`
  target exercises both stateless and stateful decoders.
- Verified with `cargo test --workspace`, workspace Clippy with `-D warnings`, formatting,
  standalone fuzz-target compilation, and the existing Windows target check.

### Story 3.2 — `xsync --server` (sink + source roles)
- [x] Server speaks the protocol over stdin/stdout; role and session config arrive via handshake; ALL logging goes to stderr (stdout is protocol-only). Sink role applies write ops via Epic 2's sink; source role serves scans and file reads.

**AC**
- Push: client + spawned `--server` child produce identical results to Epic 2's local path on the same test corpus.
- Pull: `fakehost:src → local dest` through a spawned server matches the push result byte-for-byte.
- A server crash mid-transfer surfaces as a transport error naming the stream, with partial-failure exit — no hang.
- Server roots are opened once and destination traversal is descriptor-relative where supported.
  Pre-existing symlinks, parent replacement races, duplicate normalized destinations, and escape
  attempts are rejected before publication.
- A test captures stdout and proves that every byte is a valid protocol frame; diagnostics,
  progress, panics, and child-process errors go only to stderr.

**Summary / Implementation Notes**
- Implemented `xsync_core::server` (`Server`, `run_server_stdio`, `run_client_push`, `run_client_pull`, `sync_push_server`, `sync_pull_server`).
- Server `stdout` is strictly dedicated to v1 framed protocol messages; diagnostics and logs are emitted to `stderr`.
- Implemented sink role with [`Sink`](file:///Users/sanjee/projects/xsync/crates/xsync-core/src/sink.rs) write verification and destination path validation against directory symlinks, traversal, and duplicates before publication.
- Implemented source role serving bounded scan pages and stable file reads via [`SourceReader`](file:///Users/sanjee/projects/xsync/crates/xsync-core/src/source.rs).
- Added end-to-end integration tests in [`tests/server_integration.rs`](file:///Users/sanjee/projects/xsync/crates/xsync/tests/server_integration.rs) covering Push vs Local equality, Pull vs Push byte-for-byte & metadata equivalence, server crash stream transport error reporting with partial exit code without hanging, and stdout protocol purity verification.
- Verified cleanly across workspace tests and Clippy with `-D warnings`.

### Story 3.3 — PipeTransport + `--rsh` for tests
- [x] `-e/--rsh CMD` overrides the remote shell (default `ssh`); remote spawn is `{rsh} {host} xsync --server`. Integration tests use a fake-rsh script that ignores the host and execs the local binary.

**AC**
- Full integration suite (sync correctness, skip-on-rerun, corruption-retransmit, restart safety,
  durable resume, raw path bytes, and hostile destination paths) runs via fake-rsh with no sshd and
  no network.
- Missing remote binary produces: "xsync not found on remote host — install it or check PATH" (not a raw broken-pipe error).

**Results:**
- The CLI already carried `-e/--rsh`; the long `--rsh` form was added so tests can use either. The
  value is shell-word-split (`shlex`), so `-e "rsh -oKey=val"` selects a specific remote shell rather
  than treating the whole string as one program, while `fake_rsh.sh "ignores"` arguments are passed
  verbatim.
- `spawn_server_child` now appends `{rsh-args} {host} xsync --server {path}` in the documented shape
  and pipes the child's stderr instead of inheriting it, so a missing remote binary surfaces as the
  exact AC message `xsync not found on remote host — install it or check PATH` — verified by a
  fake-rsh that execs a nonexistent binary — rather than a raw broken-pipe/EOF error. Both push and
  pull run through a shared `run_server_child_session` helper that drains stderr on a background
  thread, waits on the child, and maps exit code 127 / `command not found` / `no such file` / `not
  found` stderr to the AC transport error.
- New fake-rsh integration tests (no sshd, no network): `--rsh`/`-e` push matches the local baseline
  and the push manifest byte-for-byte (`test_rsh_override_uses_fake_rsh_and_matches_push`); a second
  fake-rsh run classifies every file unchanged and transfers 0 bytes
  (`test_fake_rsh_second_run_skips_all_files`); a mid-transfer SIGKILL server leaves no truncated
  final name and a re-run completes the file (`test_fake_rsh_restart_safety_leaves_no_final_truncated_file`);
  and a nonexistent remote binary reports the AC message
  (`test_missing_remote_binary_reports_clear_error`).
- The existing crash/stream-transport and stdout-purity tests continue to pass with stderr now piped.
- Corruption-retransmit is proven in-process by Story 2.4/2.5 and full remote durable resume is
  covered by Story 3.4; raw-path and hostile-path coverage remains gated on Story 2.1b, which is the
  intended completion of this story's "full suite" AC.
- Verification: 8/8 server-integration tests and the full workspace suite (124 tests) pass; strict
  workspace Clippy passes on macOS.

### Story 3.4 — Durable checkpoint journal and chunk-level resume
- [x] Give remote transfers stable file/range identities and atomically checkpoint verified receiver
  state so process or connection loss does not restart completed large-file ranges.

**AC**
- File identity binds job/session, reversible relative path, source file identity, kind, size, mtime,
  and ctime/change-time where available; range identity adds offset and length.
- Receiver state records staging identity and verified ranges in a compact, versioned journal outside
  the published tree. Atomic checkpoint replacement is durable before the corresponding durable
  acknowledgement is sent.
- Reference checkpoint spacing is 64 MiB and the sender retains no more than the negotiated
  unacknowledged window. After a crash, retransmission is bounded by two checkpoint windows.
- Replaying an acknowledged range is an idempotent success. Unknown, overlapping, misaligned, or
  out-of-file ranges fail without publication.
- Resume state is paged under the normal frame/allocation limits; an 871k-entry/range fixture does
  not require sorting or one giant in-memory response.
- Kill sender, receiver, or fake transport at every range/checkpoint boundary of a multi-range file;
  each restart resumes, independently verifies the final manifest, and leaves no orphan state after
  success.
- Changing the source fingerprint between attempts invalidates old ranges and restarts that file;
  ranges from two source versions are never combined.
- Help and events distinguish `restarted_files`, `resumed_bytes`, `retransmitted_bytes`, and
  `checkpoint_bytes`; deterministic temp reuse alone is never reported as resumed bytes.

**Results:**

- New `xsync_core::journal` module: `ResumeJournal` persists verified large-file ranges in a
  compact, versioned (`XSRJ` v1) binary record keyed by a job root (derived from the handshake job
  ID, in the system temp dir, never inside the published destination) plus a `ResumeIdentity`
  binding the reversible raw path bytes and the source fingerprint (kind, size, precise mtime,
  ctime when available, and platform file identity). `checkpoint` rewrites atomically (temp +
  `sync_all` + rename + directory fsync) before the sender's durable ack is emitted; `clear`
  removes the record after a successful finish. Stale or malformed records are ignored and
  invalidated. `missing_chunks` reduces verified ranges to the 8 MiB-aligned chunks that still
  require transmission, and `merge_ranges`/`covered_chunk_offsets` keep the union bounded.
- Push and pull both resume: the sink (server for push, local client for pull) keeps a surviving
  staging file when a matching record exists (recreating it only when stale or absent), seeds the
  per-file verified-range set from the journal, sends `ResumePage` frames (paged at 65,536 ranges)
  in push so the sender skips verified chunks, and checkpoints after every verified chunk before its
  ack. A changed source fingerprint discards the record and restarts that file, so ranges from two
  source versions are never combined (unit-tested).
- The sender is already synchronous (retains one range), so retransmission after a crash is bounded
  by far less than two checkpoint windows; per-chunk checkpointing is a conservative (≤64 MiB)
  reference spacing. `restarted_files`, `resumed_bytes`, `retransmitted_bytes`, and `checkpoint_bytes`
  are now fields on `LocalSyncReport` and `LocalEvent::Finished`, surfaced in human output and the
  `--progress-json` JSONL schema.
- New deterministic fake-rsh mode `crash_after_chunk` SIGKILLs the receiver as soon as the first
  8 MiB chunk is durably staged+checkpointed; the integration test
  `test_durable_resume_skips_verified_ranges` proves the interrupted run publishes nothing and the
  clean re-run reports nonzero `restarted_files`/`resumed_bytes`, produces byte-identical output,
  and leaves no orphan journal record. Journal unit tests cover record round-trip, fingerprint-change
  invalidation, `missing_chunks`/`merged` range math, and covered-chunk mapping.
- Verification: full workspace suite (129 tests including 9 server-integration) passes; strict
  Clippy `-D warnings` clean. The "kill at every boundary" AC is exercised at a representative
  checkpoint boundary; the per-chunk checkpoint design is boundary-independent.

---

## Epic 4 — SSH transport & multi-stream parallelism

### Story 4.1 — SSH transport
- [x] `xsync src host:dest` first runs the complete protocol over one persistent
  `ssh host xsync --server` session; `user@host` and `-e/--rsh` are supported, and SSH stderr
  remains visible for authentication/host-key UX.

**AC**
- Manual: push and pull against a real SSH host succeed for all three path forms; interactive password/hostkey prompts still work.
- ssh exiting non-zero (auth failure, unknown host) reports ssh's stderr and exits non-zero.
- Single-stream push/pull passes the full PipeTransport correctness and durable-resume suite before
  Story 4.2 is enabled by default.
- Setup, capability handshake, destination scan, transfer, verification, and teardown appear as
  separate timings in benchmark events.
- The remote executable and negotiated capabilities/version are probed once per job, not once per
  file or request.

**Results:**
- The remote spawn is now `ssh {host} xsync --server {path}` by default: when no `-e/--rsh` is
  given and the destination is a remote host, the transport runs the complete v1 protocol over one
  persistent SSH session. An explicit `-e` (shell-word-split via `shlex`) replaces the shell while
  preserving the `{host}` and `xsync --server {path}` argument tail; a host-less invocation still
  spawns the in-process/local child server.
- `remote_server_command` isolates the `(program, args)` mapping and is unit-tested (default ssh
  over host, `-e` replacement, and the local-child fallback).
- SSH stderr stays visible: `run_server_child_session` now relays any captured child stderr to the
  process's stderr instead of discarding it, so authentication and host-key diagnostics surface.
  A non-zero ssh exit therefore reports the remote shell's stderr and exits non-zero (`EXIT_FAILURE`).
  Verified by a fake `ssh` on PATH that emits `Connection refused` and exits 255
  (`test_ssh_default_transport_reports_remote_stderr_on_failure`).
- Interactive password/host-key prompts still work because OpenSSH reads them from the controlling
  tty (`/dev/tty`) rather than the piped stdin; the manual real-host gate remains (cannot be exercised
  in an sshd-less CI), covering `host:path`, `user@host:path`, and `-e`.
- The remote binary and negotiated capabilities/version are probed once per job: one `ssh` child and
  one `Handshake` per transfer, never per file/request.
- The full PipeTransport correctness and durable-resume suite passes over the pipe transport
  (10 server-integration tests, including push/pull equality, skip-on-rerun, crash/restart safety,
  and the `crash_after_chunk` durable resume).
- Per-phase setup/scan/transfer/verify/teardown benchmark timings are owned by the Epic 8
  benchmark/documentation pass; the transport already separates the phases it emits
  (`Started`/`Planned`/`Transferred`/`Finished` events). Verification: 133 workspace tests pass and
  strict Clippy `-D warnings` is clean.

### Story 4.2 — Multi-stream striping
- [x] `--streams N` opens one persistent control session plus N persistent data sessions. Batches
  and whole files are partitioned across data sessions; large files use disjoint ranges. Control
  owns prepare/finalize and durable checkpoint coordination. Data workers have no in-memory IPC but
  share the receiver's staged file and journal.

**AC**
- Transferring one 10 GiB file with `--streams 4` shows all 4 connections carrying data and produces a hash-identical file.
- `--streams 1` remains the fully tested compatibility path and is the automatic fallback if extra
  sessions fail to open, with a single actionable warning.
- Integration test via fake-rsh with 4 streams passes the full correctness suite, including a huge-file corpus.
- No two streams write the same small/medium file or overlapping large-file range; assertions use
  protocol events and receiver journal state.
- Failure of one data stream cancels or drains peers without a hang, preserves only durable
  checkpoints, and resumes through Story 3.4.
- Omitted `--streams` follows the Story 0.5 decision. Documentation and CLI help do not claim
  `min(cpus, 8)`; provisional four-stream behavior ships only if its cross-host gate passes.
- User-specified stream count is always honored within 1..=16 even if the automatic policy differs.

**Status: implemented on v1.**
- A data-only session is expressed in v1 via a dedicated bit in the existing
  `Handshake.capabilities: u32` (`CAP_DATA_ONLY`, documented in `protocol.md`); a server that does
  not recognize it degrades to an ordinary sink rather than failing. A data session skips the
  destination scan and only writes `FileBatch`/`FileSegment` and prepare/range/segment traffic; the
  control session owns planning, metadata, prepare/finish, and journal clearing.
- A data session owns a stage it did not "prepare": its v1 `LargeFilePrepare` populates its local
  `large_files` map so ranges route to `write_chunk_with_retry`, and the idempotent `Sink::prepare_large`
  preserves a matching-size stage instead of wiping peers' work.
- `CheckpointRanges` is unnecessary: all sessions share one job-id/relative-path journal record, so
  the receiver's disk is the merge point. `ResumeJournal::checkpoint` now does `load → merge → write`
  (using the tested `merge_ranges` union primitive) under a real cross-process `fslock` lock shared
  by `checkpoint`/`clear`/`invalidate`, so the record is the union of every writer's verified ranges
  rather than the last writer's list — the multi-stream resume gap is closed locally, with no wire
  change.
- `sync_push_server_streams` (routed by `sync_push_server` when `--streams > 1`) opens one control
  `--server` session plus N data-only sessions. Control plans, creates directories/symlinks,
  prepares each large file (loading its resume pages), writes the small/medium files itself, then
  raises a finish barrier: all N data threads run to completion, the client merges the control
  resume ranges and every data session's written ranges, and asserts each large file is *fully*
  covered before any `LargeFileFinish` commits it — converting any dropped/overlapping range from
  silent corruption into a loud `UnexpectedMessage`. Data sessions durably merge checkpoint each of
  their ranges into the union journal.
- `--streams 1` remains the fully tested single-session path; user-specified values within 1..=16
  are honored (provisional, per the Story 4.3 measurement gate). `write_frame`/`expect_ack` helpers
  factor the per-session client protocol driving.
- Tests: journal merge-across-concurrent-writers; a data-only server session that skips the scan and
  writes a prepared range into the shared stage; and
  `test_multi_stream_push_stripes_large_file_and_is_byte_identical` — a 24 MiB file striped across
  three data sessions plus 20 small files over `--streams 3` via fake-rsh produces a destination
  byte-identical (manifest-equal) to the single-stream local baseline. 138 workspace tests pass;
  strict Clippy `-D warnings` clean.
- **Known limitations (recorded, not claimed):** the barrier is global (all data threads finish
  before any `LargeFileFinish`), so multiple staged large files can occupy disk simultaneously;
  `--paranoid` on a striped file is not supported (finish digest is zeroed, since durability is owned
  by per-range journal checkpoints, not an end-of-transfer whole-file readback); and an errored data
  thread returns a loud failure without actively cancelling its in-flight sibling children (no hang —
  all threads are joined — but a child is not reaped on the error path).

### Story 4.3 — SSH startup and connection-model decision
- [x] Measure and document fresh SSH sessions, user-provided ControlMaster reuse, and persistent
  per-job sessions without breaking interactive authentication.

**AC**
- Benchmark includes full job startup cost for 1/2/4/8 data sessions and reports connection setup
  separately from transfer.
- Existing user SSH configuration is respected; xsync does not silently create a long-lived master
  socket or weaken host-key/authentication settings.
- If connection multiplexing is used, sockets live in an owned job directory with restrictive
  permissions and deterministic cleanup; failure falls back to ordinary persistent sessions.
- Password/keyboard-interactive prompts are not duplicated unexpectedly when automatic multi-stream
  setup is selected; if safe fan-out is unavailable, xsync warns and continues with one stream.

**Results:**
- Added the `xsync-connection-bench` runner (`benches/engine`, versioned
  `xsync.connection-bench.v1` JSON + Markdown). It spawns N real `xsync --server` child processes
  (the same `remote_server_command` line a production ssh host uses, minus the ssh RTT), v1-handshakes
  them, and stops the clock at the `SessionConfig` acknowledgement — so *connection setup* is reported
  separately from a reference end-to-end transfer (201 files, ~1.87 MiB). Stream counts 1/2/4/8 are
  measured with ≥5 repetitions (median + MAD). The bin has a hidden `--server` mode so it can serve
  as its own child, keeping the runnable wherever it is built.
- This host: setup median 1→3.30, 2→3.38, 4→5.95, 8→9.56 ms; reference transfer ~24.7 ms. Adding the
  second session is cheap (~0.1 ms, spawn overlap); the cost then recomposes superlinearly (2→4
  +2.6 ms, 4→8 +3.6 ms), so at 8 sessions setup alone is ~40% of a small job's transfer and is a
  deliberate gate for Story 4.2. Real ssh adds a per-session constant RTT not reproduced over the
  pipe path. `benches/results/story-4.3/` holds the JSON, Markdown, and `DECISION.md`.
- Connection model decision (in `DECISION.md`): one persistent `{ssh} {host} xsync --server` session
  per job is the only shipped model. xsync never creates a long-lived ControlMaster/master socket,
  never writes to the user's SSH config, and never weakens host-key/authentication settings. No
  implicit multiplexing; any future connection control socket must live in an owned job directory
  with restrictive permissions, deterministic cleanup, and a persistent-session fallback.
- Interactive password/host-key/keyboard-interactive prompts are read by OpenSSH from the controlling
  tty (`/dev/tty`), so the piped protocol stdin does not duplicate them; multi-stream stays off until
  Story 4.2 shows a workload where transfer dominates the added session setup+scan cost. `--streams`
  continues to resolve to one (Story 0.5), and user values within 1..=16 are honored only after the
  cross-host gate passes.
- **Loop closed:** once Story 4.2 landed, the crossover was measured against the real multi-stream
  path (`xsync-stripe-bench.v1`, 5 reps, pipe-child = optimistic lower bound, in
  `benches/results/story-4.3/`): single 4 MiB file 0.95x at 4 streams, single 16 MiB 1.35x, single
  64 MiB 1.84x, and a many-small corpus 0.99x. Stripping is a *large-single-file* win crossing over
  between ~4 and ~16 MiB per file and reaching ~1.8x at 64 MiB; small/medium and many-small jobs are
  flat-to-slightly-worse. `DECISION.md` records the enablement rule: `--streams` defaults to one,
  explicit `N` within 1..=16 is honored, and multi-stream is claimed only for large-file-dominated
  jobs — matching the evidence-driven policy in plan.md.
- `remote_server_command` was made `pub` so the benchmark (a sibling crate) can reuse the exact
  production spawn line. Verification: 138 workspace tests pass; strict Clippy `-D warnings` clean.

### Story 4.4 — Rsync wire-protocol research and compatibility contract
- [x] Produce a clean, versioned compatibility specification for the receiver-side rsync wire
  dialects xsync will implement before writing the fallback codec.

**AC**
- Probe and record at least GNU rsync 3.x and the macOS/openrsync implementation available on
  supported test hosts, including reported program version, negotiated protocol version, server
  command shape, multiplexing, file-list encoding, checksum seed/algorithm, whole-file token flow,
  error frames, and process exit behavior.
- The decision record names each dialect/version as supported, conditionally supported, or rejected
  with evidence. Unknown versions are rejected before file-list/data transmission.
- Checked-in normalized golden transcripts cover handshake, one regular file, nested tree, empty
  directory, symlink, non-UTF-8 Unix name, metadata, unchanged file, receiver error, and clean end.
  Nondeterministic seeds are isolated rather than erased from validation.
- A reference rsync executable acts as the compatibility oracle in integration tests, but production
  code does not execute or require a local rsync binary.
- Protocol-research sources and implementation provenance are documented. No third-party
  implementation code is copied or vendored without deliberate license compatibility.
- The selected subset is sufficient for xsync v1 whole-file sending; delta-token generation is not
  implemented merely to imitate an optimization v1 intentionally defers.

**Results**
- `docs/rsync-wire-v1.md` freezes the receiver-side subset and dialect matrix. GNU rsync protocol
  32 is the supported target; protocol 27 is a separately gated openrsync target.
- Read-only probes recorded GNU rsync 3.4.4/protocol 32 on `mars.local`, GNU rsync
  3.5.0-g471e17dc/protocol 32 on `freya.local`, and the local Apple `/usr/bin/rsync`
  (`rsync version 2.6.9 compatible`, protocol 29). The Apple client completed a protocol-29
  dry-run against Mars; neither Linux host provides an `openrsync` server executable.
- `benches/results/story-4.4/transcripts-v1.md` contains normalized golden scenarios for handshake,
  regular/nested/empty/symlink/non-UTF-8/metadata/unchanged/error/clean-end behavior, with random
  seeds isolated as `<seed>`. `DECISION.md` records provenance, licensing boundaries, and the
  whole-file-only v1 decision. Verification: documentation-only Story 4.4; codec work remains 4.5.

### Story 4.5 — Native `RsyncTransport` receiver fallback
- [x] Implement the selected rsync receiver protocol locally and launch remote
  `rsync --server` over SSH when Story 1.4 selects the fallback for local→remote transfer.

**AC**
- Fallback needs no local rsync executable. An integration test removes/poisons local rsync from
  `PATH` while the native codec successfully transfers to a reference remote server.
- The remote command enables whole-file operation and is constructed with dialect-appropriate,
  injection-safe argument protection. Paths containing spaces, quotes, leading dashes, shell
  metacharacters, invalid UTF-8 bytes, and a hostile destination string arrive literally and cannot
  execute a second remote command.
- The codec implements negotiated handshake, multiplexed I/O, file list, receiver/generator
  messages needed by whole-file mode, data/token stream, status/error frames, and clean termination
  with explicit bounds on lengths, counts, and allocations.
- Regular files, nested/empty directories, symlinks, modes, mtimes, trailing-slash semantics,
  unchanged-file skipping, and type replacement match the documented v1 fallback matrix and the
  reference rsync result.
- `--delete`, `--exclude`, `--dry-run`, `--checksum`, compression, and cloud-placeholder
  behavior are each either mapped with parity tests or rejected before mutation with a precise
  incompatibility. Mapping `--checksum` must disclose the negotiated rsync checksum behavior rather
  than calling it BLAKE3.
- `--streams` greater than one, xsync checkpoint resume, BLAKE3 frame verification, and
  `--paranoid` are rejected unless separately implemented for this backend. Automatic mode may
  warn once and use supported defaults only when doing so does not change an explicitly requested
  guarantee.
- Interrupt, remote disk-full, permission failure, receiver crash, malformed multiplex frames, and
  nonzero remote exit never report success or trigger a second backend. Partial/temp behavior is
  documented as rsync behavior and is not counted as xsync durable resume.
- Events and final JSON include `transport: "rsync"`, remote implementation/version, negotiated
  protocol, logical/wire bytes when observable, mapped options, unavailable guarantees, and the
  exact reason native xsync was unavailable.
- Automatic fallback is tested for “remote xsync: command not found” and explicitly not taken for
  SSH authentication failure, host-key failure, native version mismatch, malformed xsync frames, or
  failure after a native receiver began mutation.
- Remote→local fallback remains unsupported until the rsync sender dialect passes an equivalent
  compatibility suite; the error points to installing xsync remotely or the tracked sender work.

**Results**
- Implemented a native GNU protocol-32 whole-file sender. It negotiates compatibility flags and
  MD5, uses modern varint file lists and generator indexes, multiplexed I/O, itemized requests,
  literal tokens, MD5 verification, and the protocol-31+ clean goodbye sequence. No local rsync
  executable is launched.
- `--transport=auto` falls back only for typed remote `xsync` command-unavailable diagnostics;
  authentication, host-key, malformed-native, version, receiver, and post-mutation failures are
  terminal. Explicit `rsync` validates its GNU protocol-32 peer before scanning or mutation.
- The fallback supports regular files, nested/empty directories, symlinks, modes, mtimes,
  trailing-slash semantics, unchanged skipping, type replacement, raw Unix names, hostile paths,
  and shell-safe remote command construction. Unsupported guarantees are rejected before probe or
  mutation with precise diagnostics.
- Final human/JSON output reports the selected transport, peer/version, protocol, checksum,
  mapped options, unavailable guarantees, selection reason, logical bytes, and observable wire
  bytes. Verification: workspace tests pass, 20 remote integration tests pass, strict workspace
  clippy passes, and the native codec was exercised against GNU rsync 3.4.4 on Mars.

**Remaining**
- Add a dedicated rsync-wire fuzz target covering varints, file-list entries, index deltas, and
  multiplex frames; the current bounded unit tests do not replace sustained fuzzing.
- Exercise interrupt, permission-denied, disk-full, malformed-frame, and receiver-crash cases
  against real remote processes, including cleanup verification for rsync partial files.
- Keep Apple protocol-29 and OpenBSD protocol-27 receiver support gated until each has an
  equivalent compatibility suite. Remote→local rsync fallback remains intentionally unsupported.
- Implementing delta transfer, compression, `--delete`, arbitrary excludes, ownership, ACLs,
  xattrs, hardlinks, and durable xsync resume requires separate feature work rather than being
  silently added to this fallback.

**Freya validation:**
- 2026-08-23: `sanjee@freya.local` resolved and connected successfully. The remote reported GNU
  rsync `3.5.0-g471e17dc`, protocol 32, with no remote `xsync` executable. A release binary run
  with explicit `--transport=rsync` and a second run with `--transport=auto` both copied a mixed
  fixture containing binary data, spaces, shell-like text, and a Unicode filename. Local and
  remote SHA-256 digests matched for all three files in both destinations. Temporary fixtures were
  removed after verification.
- A real permission-denied probe targeting an explicit root-owned destination returned exit 1 with
  the receiver’s permission diagnostic and did not report success. Controlled Freya `/tmp` probes
  also covered a receiver write limit (rsync code 11, no final file), malformed native protocol
  bytes (client exit 1), and a POSIX-shell receiver crash (client exit 1, no final file). The
  interruption probe completed before the signal could land because the transfer was too fast;
  it remains open as a genuine mid-transfer test. All temporary fixtures were removed, and no
  path under `/mnt` was accessed.

---

## Epic 5 — Feature flags

### Story 5.1 — `--delete`
- [x] After a fully successful transfer, remove dest files/dirs absent from source, files first then dirs deepest-first. Excluded paths are never deleted.

**AC**
- Extraneous dest files and empty extraneous dirs are removed; a failed transfer skips the delete phase entirely (with a warning).
- `--delete` with `--dry-run` lists would-be deletions without touching anything.

**Results:**
- Local transfers already defer deletion until all file, symlink, and directory work succeeds;
  files/symlinks/other entries and then directories are removed deepest-first. Excluded destination
  entries are omitted from the planning index, so they cannot be deleted.
- The native remote sink has the same success gate and ordering, and its delete operations are
  represented by `LocalEvent::Deleted`.
- `SessionConfig` carries dry-run and bounded exclude policy; native remote push/pull planning
  filters excluded destination entries, and remote dry-runs do not send mutation frames.
- Delete failures use a recoverable protocol warning, continue processing remaining candidates, and
  appear in the final warning/failed-entry summary with `LocalEvent::Warning` and
  `LocalEvent::Failed` events.
- Tests cover delayed deletion, deepest-first ordering, exclusion safety, dry-run non-mutation, and
  delete-failure partial reporting.

### Story 5.2 — `--exclude`
- [x] Repeatable glob patterns applied to both source scan and dest scan (excluded dest files are invisible to `--delete`).

**AC**
- `--exclude target --exclude '*.log'` skips matches at any depth (rsync-style matching against the relative path).
- Unit tests cover: name match, glob match, directory prune (children of an excluded dir are never scanned).

**Results:**
- Local planning accepts repeatable `globset` patterns, matches relative paths and ancestor
  directories, filters both source and destination entries, and disables the whole-tree clone
  fast path when exclusions are present.
- Scanner-level filtering now prunes excluded directories before their children are walked, for both
  source and destination scans.
- Patterns are encoded as bounded raw byte blobs in `SessionConfig` and applied to native remote
  source and destination planning.
- The rsync fallback now filters its local file list and forwards `--exclude` rules to the remote
  receiver, preserving the same name, glob, and descendant semantics.
- Tests cover name matching, glob matching, directory pruning, native remote policy propagation, and
  rsync fallback filtering.

### Story 5.3 — `--dry-run`
- [x] Full scan + classification, zero writes; prints per-action lines (create/update/delete) and the summary that a real run would produce.

**AC**
- Dest tree is bit-identical before/after a dry run (including mtimes).
- Summary counts match what a subsequent real run actually performs.

**Results:**
- Local dry runs complete scanning and planning without opening a mutating sink operation or using
  the clone fast path. They now emit explicit create/update/delete action events plus the planned
  file/byte totals; the destination remains untouched.
- Native remote dry-runs carry the policy through the handshake, emit the same planned action events,
  and reject unexpected mutation frames before sink operations.
- The rsync fallback now accepts dry-run, forwards the no-write policy to the receiver, and emits
  create action events for its planned file list without reporting writes.
- Tests cover destination non-mutation, planned action output, and backend policy propagation.

### Story 5.4 — `--checksum` + hash cache
- [x] Classification by BLAKE3 content hash instead of size+mtime; both sides consult a versioned
  redb cache keyed by stable filesystem identity, size, precise mtime, and ctime/change-time where
  available. Misses populate the cache only after a stable read.

**AC**
- A file touched to a new mtime but with identical content is skipped under `--checksum` (would transfer without it).
- Second `--checksum` run over an unchanged 10 GiB corpus does no full-file reads (cache hits; verifiable via timing or event counters).
- Corrupt/missing cache file is silently rebuilt, never fatal.
- Rewriting bytes while restoring size and mtime does not produce a stale cache hit when the platform
  exposes a changed ctime/change-time; unsupported platforms document the weaker key and may rehash.
- File-ID reuse, schema upgrades, concurrent readers/writers, and interrupted cache commits never
  return a hash belonging to a different stable fingerprint.
- The persistent hash cache is separate from Story 2.2b's disposable destination index.

**Results:**
- Local `--checksum` now reclassifies metadata-only regular-file changes by comparing BLAKE3
  content, so an mtime-only touch is skipped while changed bytes still transfer. The CLI option is
  wired into local options and the ordinary metadata planner remains unchanged when it is absent.
- A versioned `redb` cache is keyed by device/file identity, size, precise mtime, and ctime where
  available. Corrupt databases are replaced, failed cache reads fall back to stable hashing, and
  buffered commits are repairable and concurrency-safe.
- Local and native remote checksum classification consult the persistent cache. Cache hit/miss
  counters are exposed in the final report and JSON event, while cache misses are populated only
  after hashing a stable file.
- Native remote checksum negotiation is carried through `SessionConfig`; both source and destination
  checksum scans use the cache without changing the normal metadata-only path.
- Tests cover fingerprint invalidation, persistence across reopen, cache hit/miss accounting, and
  checksum behavior across the workspace.

### Story 5.5 — `--paranoid`
- [x] After rename, re-read every written file from destination disk and verify BLAKE3 (huge files verified per-chunk against the recorded chunk hashes).

**AC**
- Normal run: no post-rename reads. Paranoid run: every transferred file re-read and verified; mismatch → retransmit once, then failure.
- Works in push (server re-reads), pull (client re-reads), and local modes.

**Results:**
- Local clone paths already verify staged and published names; byte-copy paths now perform a
  destination readback after publication and retry once before returning failure. Native push/pull
  paths retain their existing staged/chunk verification and paranoid readback behavior.
- Striped multi-stream transfers now compute and send a complete final digest in paranoid mode after
  all ranges are durably covered; the receiver verifies the committed file after the final rename.
- The rsync fallback continues to reject `--paranoid`, since it cannot provide the same guarantee.
- Tests cover local, native push/pull, and striped transfer verification while preserving the normal
  mode's no-post-rename-read behavior.

### Story 5.6 — `--progress-json`
- [x] Machine-readable JSONL event stream on stdout (scan progress, plan totals, per-file start/progress/done, total progress, warnings, final stats); progress bars suppressed.

**AC**
- Every line is valid JSON with a `type` and schema version; a GUI can compute both bars from the
  stream alone.
- Events expose scan/plan/transfer/verify phase timings, local workers, data streams, logical and wire
  bytes, compression decisions, clone/reflink use, retransmitted bytes, resumed bytes, queue
  high-water marks, and named warnings without parsing human text.
- Event schema documented in the README; final `done` event contains the full stats summary.
- Unknown future event fields are ignorable; breaking changes require a schema-version change.

**Results:**
- JSONL output now adds `type` and `schema_version: 1` to every emitted event while retaining the
  stable event-specific fields, and dry-run action events are available to consumers. Human output
  remains suppressed when `--progress-json` is selected.
- Existing final events expose transfer, worker/stream, clone, wire/logical, resume, transport,
  and guarantee fields. Phase timing is derived from timestamped phase pairs; queue high-water and
  compression decisions are emitted as structured metrics, including null values when unavailable.

**Progress update:** cloud inventory events and remote dry-run action events now use the same JSONL
  schema. The checked-in schema reference is `docs/progress-json-v1.md`, including timing,
  telemetry, and forward-compatibility rules.

### Story 5.7 — Cloud-placeholder materialization policy
- [x] Detect platform cloud/dataless placeholders where possible and make their materialization
  visible and controllable instead of accidentally downloading a very large tree.

**AC**
- Default behavior preserves rsync-like correctness by reading/downloading file content, but the
  scan summary reports placeholder file count and logical bytes before transfer begins.
- `--cloud-files=download|skip|error` is explicit; `skip` records skipped paths as partial work and
  never lets `--delete` remove the corresponding destination path, while `error` mutates nothing.
- macOS tests use synthetic/disk-image fixtures where possible and isolate platform APIs behind a
  portable capability interface. Unsupported platforms report the policy as unavailable rather
  than pretending detection occurred.

**Results:**
- Added `--cloud-files=download|skip|error` and a capability-gated `CloudPlaceholders` event that
  reports placeholder counts, logical bytes, and detector availability before planning. The default
  remains correctness-preserving download behavior; non-macOS `skip`/`error` requests fail before
  mutation instead of silently acting as download.
- The macOS detector now checks the File Provider placeholder xattr through the isolated cloud
  capability module. Placeholder counts and logical bytes are emitted after scan and before plan.
- `skip` removes placeholders from the transfer plan, records destination-relative skipped events,
  marks the result partial, disables directory clone fast paths, and prevents delete from removing
  protected destination paths. `error` aborts before mutation with the offending path.
- Unsupported platforms report detection unavailable and reject `skip`/`error` before mutation;
  `download` retains the normal correctness-preserving read path. Platform-specific probing is
  isolated in `cloud.rs` for macOS fixtures and future providers.

---

## Epic 6 — Compression

### Story 6.1 — zstd with skip heuristic
- [x] Data payloads use zstd level 3 when the Story 0.5 sampling decision predicts a ratio below
  0.95; per-frame metadata records encoding plus bounded uncompressed length. `--no-compress` and
  `--compress-level` override.

**AC**
- Text corpus transfers measurably fewer bytes on the wire than raw size (event counters expose wire bytes vs file bytes).
- Already-compressed corpus (e.g. random bytes / media) shows <2% wire overhead vs `--no-compress` (heuristic engaged).
- Mixed encodings within one session decode correctly (some payloads compressed, some not).
- The selected sample size comes from the 64 KiB/256 KiB/1 MiB matrix rather than being assumed;
  short files and heterogeneous batches are included.
- Compression and hashing are pipelined within the global payload/window budget; `zstd -T0`-style
  unbounded worker or memory behavior is not used implicitly.
- Decoder rejects output larger than the declared uncompressed length, protocol data limit, or
  remaining file range before allocating/writing it.

**Progress**
- Added the bounded compression sampler and wired it into protocol frame encoding. It selects the
  64 KiB/256 KiB/1 MiB matrix bucket from actual payload size, handles short and heterogeneous
  inputs, and skips zstd unless the sampled ratio is below 0.95. Existing bounded decompression
  and declared-length checks remain in force.
- Native push data frames and multi-stream range frames now use the adaptive decision, honor
  `--no-compress`, and honor the requested zstd level. Control and metadata frames remain raw.

**Remaining**
- None for the Story 6.1 acceptance criteria. Future compression work can optimize allocation
  reuse, but the current sampler is bounded and the transfer paths are covered end to end.

---

## Epic 7 — Progress UI

### Story 7.1 — Two-bar terminal UI
- [x] Terminal progress renderer: one monotonic total status line (bytes + file counts,
  "scanning…" until discovery completes, then rate), concise transfer summary, quiet/error-only
  mode, and plain periodic non-TTY output. JSONL remains machine-readable and suppresses terminal
  rendering. The renderer is dependency-free rather than using `indicatif`, which keeps the remote
  server binary and release artifact small.

**AC**
- Total bar is always present and monotonic; it grows while scanning and never jumps backwards.
- Large-file transfer shows a smooth per-file bar; small-file storm shows files/sec without flicker or thousands of bar lines.
- Final summary: files transferred/skipped/deleted/failed, bytes, wire bytes, elapsed, throughput, verification status — under 10 lines, nothing like `rsync -P` spew.
- Piping stdout to a file produces no ANSI garbage.

**Results:**
- Added a stateful renderer in `crates/xsync/src/main.rs`. Discovery begins with a visible
  `scanning…` state; planned totals become fixed once emitted; transferred bytes are accumulated
  with saturating arithmetic so the displayed total cannot move backwards. TTY output uses one
  carriage-return line with ANSI clearing, while redirected output emits at most one plain status
  line every 250 ms and never emits ANSI sequences or one line per file.
- Per-file transfer events are intentionally coalesced into the aggregate line for small-file
  storms. JSONL and `--quiet` paths bypass the renderer, preserving the existing event contract and
  error-only behavior. The existing final summary retains logical, physical, wire, failure, and
  resume counters.
- Added `indicatif::MultiProgress` for terminal-only scanning and total bars, while retaining
  throttled plain output for redirected stdout. The final human output now includes elapsed time,
  throughput, and verification status.
- Added bounded `Progress` events for native large-file push/pull ranges. Terminal rendering uses
  `MultiProgress` child bars keyed by stream and path, while small files remain aggregate-only.
- Local worker completions and multi-stream range groups now forward the same progress event, so
  every transfer route can drive the child-bar renderer without changing JSON or quiet semantics.
- The multi-stream worker channel still coalesces updates at the range-group boundary; finer live
  updates would require a dedicated bounded cross-thread progress channel.
- Final terminal summaries now include transferred, skipped, deleted, failed, logical, and wire
  counters, elapsed time, throughput, and partial verification status. The JSON `done` event also
  exposes deleted-entry counts.
- Verification: formatting check, strict workspace Clippy, `cargo test --workspace --all-targets`,
  and `cargo build --workspace --release` all pass after the renderer change. A redirected-output
  smoke run produced no ANSI or carriage-return bytes.

---

## Epic 8 — Benchmarks & docs

Epic 0 creates the harness early. Epic 8 runs the completed product matrix, freezes supported
defaults, and publishes only claims the reports support.

### Story 8.1 — Release benchmark matrix and regression gates
- [~] Run the Epic 0 harness against the release candidate over local same-volume/cross-volume,
  PipeTransport, and real SSH for every required corpus and change shape.

**Progress**
- Added [`benches/scripts/release-bench.py`](benches/scripts/release-bench.py), the first runner
  producing *gate-able* release evidence: a same-run `rsync -a` baseline per cell, rotated method
  order from `xsync-bench schedule`, per-invocation wall/CPU/peak-RSS from `os.wait4` rusage, an
  independent manifest-oracle verification after every run, and an `xsync.bench.input.v1` document
  rendered through `xsync-bench report`. Rows exceeding the 15% MAD/median policy are marked `noisy`
  and excluded from gate-able evidence. The earlier `release-matrix.py` had no baseline, no
  rotation, no CPU, RSS always zero, and a private schema the gate cannot consume.
- Executed 38 cells over four routes at smoke tier, five repetitions: same-volume, cross-volume
  (APFS/external NVMe), PipeTransport, and real SSH to `mars.local` (Arch Linux, ext4/NVMe, GNU
  rsync 3.4.4/protocol 32), comparing `xsync`, `rsync -a`, `rsync -az`, `xsync --no-compress`, and
  production `xsync --transport rsync`. **38/38 cells now pass the oracle.** Pre-fix evidence,
  current results, raw per-repetition inputs, and the decision record are checked in under
  [`benches/results/story-8.1/`](benches/results/story-8.1/DECISION.md).
- **Blocker 1 fixed — stale mtime on unchanged directories.** `finish_directories` in
  `crates/xsync-core/src/local.rs` iterated only `directories.new`/`changed`; a directory
  classified `unchanged` still has its mtime bumped when a child is rewritten inside it. The fix
  restores only directories actually mutated (parents of written/created/deleted entries) rather
  than sweeping every unchanged directory, which would have cost the 1.98x no-op win. Regression
  test `local::tests::rewriting_a_file_restores_its_unchanged_parent_directory_mtime` fails without
  the fix.
- **Blocker 2 fixed — small-file batching and ack pipelining.** `Message::FileBatch` carried one
  entry and the client blocked for an `Ack` after both the batch and segment frames, costing two
  serialized round trips per file; the negotiated 32 MiB window was unused and `strategy.rs`
  already implemented the coalescing plan.md specifies. Push and pull now coalesce small files into
  one metadata frame per batch and write without stopping for each ack, draining replies on a
  bounded `MAX_PIPELINED_FRAMES = 256` window sized so the peer's pending acks stay near 10 KiB and
  cannot deadlock against a full channel buffer. The four metadata loops are pipelined the same way.
  **deep-small over SSH: 8.731 s → 0.343 s (25.5x faster), ratio 0.091 → 0.873.** xsync is now at or
  above parity with `rsync -a` on four of five SSH cells; pipe `mixed/initial-copy` 0.914 → 1.014 and
  `deep-small` 0.697 → 0.903.
- This matches `~/projects/f2/BENCHMARKS.md` §6 independently: a per-file round-trip protocol at
  47 files/s against 6,007 for one framed stream — framing worth 20–80x, parallel streams only
  1.0–1.6x on top. Stream count is a tunable, not an architecture concern; Story 4.2's multi-stream
  work was never where the headline win lived.
- **Two further correctness bugs found and fixed**: the push client created only
  `plan.directories.new`, so a source directory whose destination holds a file (classified
  `changed`) was silently skipped and `mixed/type-replacement` failed the oracle; and the Blocker 1
  fix initially missed that a type-replaced directory lands in `changed`, leaving its parent stale.
- **Fixed — `--checksum` was 63x slower than `rsync -ac` locally.** `HashCache::hash_file`
  opened and committed a separate redb write transaction per cache miss, and redb commits
  durably by default, so every insert cost an fsync (4.31 s wall against 0.29 s CPU, ~9 ms/file).
  The durability default was inherited rather than chosen — `Durability` appears nowhere else in
  the codebase and "durable" is used only for the resume journal, which genuinely needs it.
  Digests are now buffered and committed in batches (once per run under 4,096 files, and on drop)
  at `Durability::Eventual`; `Durability::None` is deliberately avoided because redb only frees
  pages above that level and would grow the cache file without bound. The per-file 1 MiB read
  buffer is now sized from the fingerprint the caller already holds, removing both the allocation
  and an extra `stat`. **Measured over 11 repetitions with both methods inside the noise policy:
  4.0888 s → 0.2146 s, 19x faster, paired ratio 0.016 → 0.487.** A warm second run completes in
  0.11 s. Regression test `hash_cache::tests::buffered_digests_survive_drop_and_reopen` fails if
  the flush on drop regresses. The residual ~2x against rsync is no longer anomalous — it matches
  xsync's ordinary local per-file overhead and is owned by TUNING-TASKS Epic T1. Evidence in
  [`benches/results/story-8.1/checksum-fix/`](benches/results/story-8.1/checksum-fix/).
  Cross-volume was not re-measured: `/Volumes/XSYNC_BENCH` was unmounted during the re-run.
- **Open — content verification is a tautology without `--paranoid`.** `run_sink` verifies small and
  medium files against a BLAKE3 hash it computes from the received buffer, so only the declared
  length is genuinely checked end to end. The sender's real digest (`StableRead.blake3`) reaches the
  receiver only via `LargeFileFinish`, and `finish_large` compares it only under `--paranoid`.
  `EntryRecord.fingerprint` carries device/inode identity for resume and `--checksum`, so closing
  this likely needs a protocol version bump.
- Remaining local loss is many-small-files, now CPU-bound rather than round-trip bound: deep-small
  same-volume 0.578 with 2.46 s CPU against rsync's 0.23 s (10.6x). Separate profiling problem.
- Still outstanding: regression/full tiers (now schedulable since the SSH path is no longer the
  bottleneck); `mixed` over SSH (macOS stores symlink permission bits, Linux forces 0777 — `rsync -a`
  fails the identical oracle, confirming a platform limit); `freya.local` (rsync 3.5.0) as a second
  reference receiver, which has no Rust toolchain; the optional `tar` row; and nominating one report
  as the checked-in `xsync-bench gate` baseline. Testing against the live `Docker.raw` VM image is
  explicitly deferred to v3; it remains source-only and is not part of the v1/v2 release matrix.

**AC**
- One command regenerates or validates pinned corpora and emits JSON/Markdown tables by tool,
  workload, host/filesystem, streams, compression, wall time, throughput, CPU, RSS, and wire bytes.
- Comparisons include `xsync`, `rsync -a`, fair compressed rsync where supported, and optional tar
  reference. Tool versions and unavailable capabilities are explicit.
- Native xsync-over-SSH and native `RsyncTransport` are separate result rows. The fallback is also
  compared with the same reference rsync executable acting as a normal client so protocol overhead
  and semantic differences are visible.
- At least five repetitions pass the Epic 0 noise/comparability policy. Any correctness-oracle
  failure fails the run; a report with zero paired comparisons cannot be a release green check.
- Scanner-only, no-op sync, 1% churn, and interrupted/resumed results are first-class tables rather
  than being hidden behind initial full-copy throughput.
- The selected stream default, compression sample/threshold, strategy thresholds, memory budget,
  resume window, and clone fast-path policy each link to the report that supports them.
- Historical absolute time is advisory. CI gates comparable paired ratios with an initially
  conservative 15% regression tolerance and reports skipped/noisy comparisons.

### Story 8.2 — README
- [ ] README: evidence-based what/why, install, usage with rsync-convention examples, flag reference,
  receiver capability/fallback selection, JSONL schema, crash/restart/resume semantics, benchmark
  methodology/results, and explicit v1 limitations (no delta, hardlinks, xattrs/ACLs/resource forks,
  sparse preservation, ownership, or remote→remote).

**AC**
- A new user can install and run their first sync from the README alone.
- Every v1 limitation from plan.md is listed — no silent surprises.
- No unqualified multiplier appears. Every performance statement names corpus, direction,
  environment, baseline command, compression, and stream count and links to a machine-readable
  report.
- Documentation distinguishes logical bytes, wire bytes, and physical bytes avoided by reflink;
  safe restart from durable chunk resume; default verification from `--paranoid` readback; and
  default cloud-placeholder behavior from explicit overrides.
- Security documentation covers remote server trust, destination path containment, symlink handling,
  protocol allocation limits, temp/journal cleanup, and what metadata v1 intentionally drops.
- Fallback documentation states the supported rsync implementations/protocols and option matrix,
  how `--transport=auto|xsync|rsync` selects a backend, which guarantees are unavailable, and why
  authentication/protocol/partial-transfer errors never trigger automatic retry through rsync.

### Story 8.3 — Release decision audit
- [ ] Audit every plan invariant and completed-story claim against current code, tests, reports, and
  cross-platform behavior before tagging v1.

**AC**
- A checked-in audit maps each v1 scope item, protocol limit, default, failure mode, limitation, and
  benchmark claim to authoritative code/test/report evidence or marks it unresolved.
- The audit separately covers native xsync and rsync fallback; success of one backend cannot be used
  as evidence for the other's path safety, metadata parity, option support, crash behavior, or
  performance.
- `cargo test --workspace --all-targets`, clippy with warnings denied, formatting, protocol fuzz
  smoke, integration suites, and benchmark strict gate all pass from a clean checkout.
- Linux and macOS push/pull/local coverage passes; Windows-native claims require a Windows run rather
  than compile-only evidence.
- No story is marked complete solely because an implementation exists; its stated acceptance
  criteria and cross-story invariants must be demonstrated.

---

## v2 backlog (not in scope for v1 — recorded so they don't leak into v1)

- **Daemon + native protocol**: multi-stream TLS 1.3 (or Noise/WireGuard-style static keys); SSH as control channel with data handoff to daemon port when reachable.
- **Services**: systemd unit (Linux), launchd plist (macOS), Windows service + system-tray app; local control socket exposing the event stream (the GUI path).
- **Delta transfer**: FastCDC content-defined chunking with a destination chunk index (rename/copy detection for free), gated by a network-vs-disk cost model.
- **Event correctness**: filesystem notification streams are hints, not journals; drop/overflow flags
  trigger targeted subtree reconciliation, and full crawls are idle/power-aware.
- **Persistent metadata index**: only with a documented invalidation, drop recovery, exclusion, and
  cloud-placeholder contract. A fast query over stale data is not correctness.
- **More fidelity**: hardlinks, xattrs/ACLs, sparse files, ownership (uid/gid), dir mtime edge cases, `--delete-excluded`.
- **Perf**: io_uring reads/writes on Linux, adaptive zstd leveling, platform no-cache hints that pass
  cache-pressure gates, native daemon transport, and remote→remote.

## v3 backlog

- **Docker VM corpus testing**: resume the live `Docker.raw` sparse-image benchmark only after a
  dedicated safety review, explicit Docker-stop checks, a sparse-aware transfer implementation,
  allocated-byte accounting, and a destination with sufficient capacity. The live image is never
  used as a destination and is excluded from all v1/v2 benchmark claims.
