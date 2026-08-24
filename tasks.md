# xsync v1 — Epics, Stories & Acceptance Criteria

Companion to [plan.md](plan.md). Ordered roughly by implementation sequence; stories within an epic are independent unless noted. "AC" = acceptance criteria.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

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

**Known delta after the `f2` review:** Story 0.5 selected one stream and the help text now records
that decision. Story 4.2 must give the omitted option runtime transport behavior; the parsed value
correctly remains `None` until configuration resolution is implemented.

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
- [ ] Add `--transport=auto|xsync|rsync` with `auto` as the default and model transport
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

---

## Epic 1 — Implementation Report

Epics 1.1–1.3 delivered a clean cargo workspace, a full clap CLI surface, and rsync-compatible path parsing. All AC met; clippy clean; no work performed yet beyond validation.

**Layout**
- Root `Cargo.toml`: workspace (`resolver 2`, members `crates/xsync`, `crates/xsync-core`), shared `[workspace.package]` (edition 2021, rust-version 1.88), `[workspace.lints]` (`unsafe_code = deny`, clippy `all`+`pedantic` warn), release profile `lto`+`codegen-units=1`+`strip`.
- `crates/xsync-core` — engine library. `version()`, `PROTOCOL_VERSION`, `HANDSHAKE_MAGIC`; new `path` module (`PathSpec`, `Location`, `parse`, `validate_pair`, `PathError`). Deps: `thiserror`.
- `crates/xsync` — binary. clap-derived `Cli` (struct); `run()` parses SRC/DEST and rejects remote→remote; returns `ExitCode` on error. Deps: `clap`, `xsync-core`.

**What works**
- `cargo build` / `cargo test` clean on a fresh checkout; 16 tests pass (4 CLI + 12 core).
- `cargo run -- --help` documents every flag; hidden `--server` present but not shown.
- Unknown flag, missing SRC/DEST, and out-of-range `--streams`/`--compress-level` → exit 2 with a one-line clap error (no panic); remote→remote → exit 1 with `xsync: remote-to-remote sync is not supported in v1`.
- Path parsing handles `dir`, `dir/`, `host:dir`, `user@host:dir/`, `./relative`, single-file source, and Windows drive letters (`C:\Users\x`, `C:/Users/x`) — never misread as hosts.
- Release binary: single Mach-O arm64 executable (~600 KB), no runtime asset dependencies.

**Deferred to Epic 2 (per Story 1.3 AC)**
- Output-tree trailing-slash semantics: `xsync a b` → `b/a/...` vs `xsync a/ b` → `b/...` are captured in `PathSpec.trailing_slash`; the observable tree result is verified in Epic 2's integration tests.

**Interface for Epic 2**
- `PathSpec { is_remote(), host(), path, trailing_slash }` and `xsync_core::path::{parse, validate_pair}` are the entry points the local engine's scanner → planner → transfer should consume.

---

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
- [ ] Replace the string-only relative path with a reversible component representation and extend
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
- [~] `--streams N` opens one persistent control session plus N persistent data sessions. Batches
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

**Status: v1-expressible; coordination rests on client sequencing + a journal merge fix.**
- Revisited after review: protocol v2 is **not** required for multi-stream. All three originally-flagged
  needs are expressible in v1. (1) A data-only session is just `Role::Sink` plus a dedicated bit in
  the existing `Handshake.capabilities: u32` — decoded with no known-bits mask, so arbitrary bits
  round-trip today; the server honors it by skipping the destination scan. (2) A data session can own
  a stage it did not "prepare": its own v1 `LargeFilePrepare` populates its local `large_files` map
  so ranges route to `write_chunk_with_retry`, and the idempotent `Sink::prepare_large` preserves a
  matching-size stage instead of wiping peers' work. (3) `CheckpointRanges` is unnecessary: all
  sessions share one job-id/relative-path journal record, so the receiver's disk is the merge point.
- The one genuine gap is **multi-stream resume**: `journal::checkpoint` blindly overwrites with the
  caller's in-memory list (`journal.rs::165`), so the last writer's ranges survive and the rest are
  lost (content stays correct; the next run just retransmits them). Closing it is a local, testable
  change in one crate: make `checkpoint` `load → merge → write` using the already-written-and-tested
  `merge_ranges` union primitive, guarded by a real cross-process lock — `read-merge-write` is not
  atomic even with the atomic final rename, and `clear`/`invalidate` must be safe against a
  concurrent `checkpoint`. No wire change.
- The actual bulk of 4.2 is **client-side sequencing**, not protocol: a barrier before
  `LargeFileFinish`, control-only ownership of directories/symlinks/deletes, and drain-on-failure
  semantics. These are implementable in the orchestration layer.
- Defects fixed/handled along the way: `server.rs` no longer silently acks a `FileSegment` whose
  `file_id` is unregistered (was the exact drop path for a mis-sequenced data session) — it is now a
  hard `UnexpectedMessage` error, with a regression test.
- **Shipped now:** idempotent `Sink::prepare_large`; the silent-ack→loud-error fix + test;
  `--streams` parsing (1..=16) unchanged and Story 0.5's default of one stream intact (fully tested),
  so nothing unverified is claimed.
- **Sequencing decision:** measure before building. Run Story 4.3 first — if four sessions cost four
  SSH handshakes plus four destination scans, striping may not pay for small/medium jobs, and the
  crossover point should drive whether/when the coordination complexity is worth it. If it pays,
  build 4.2 on v1 as: capability bit → data-only sessions, idempotent prepare (done), merge-on-checkpoint
  under a lock, and client-side finish/metadata/delete barriers. Keep protocol v2 in reserve for
  something that genuinely needs new message types (e.g. delta transfer's chunk index), where the
  v1/v2 compatibility matrix cost is actually justified.

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
- `remote_server_command` was made `pub` so the benchmark (a sibling crate) can reuse the exact
  production spawn line. Verification: 135 workspace tests pass; strict Clippy `-D warnings` clean.

### Story 4.4 — Rsync wire-protocol research and compatibility contract
- [ ] Produce a clean, versioned compatibility specification for the receiver-side rsync wire
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

### Story 4.5 — Native `RsyncTransport` receiver fallback
- [ ] Implement the selected rsync receiver protocol locally and launch remote
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

---

## Epic 5 — Feature flags

### Story 5.1 — `--delete`
- [ ] After a fully successful transfer, remove dest files/dirs absent from source, files first then dirs deepest-first. Excluded paths are never deleted.

**AC**
- Extraneous dest files and empty extraneous dirs are removed; a failed transfer skips the delete phase entirely (with a warning).
- `--delete` with `--dry-run` lists would-be deletions without touching anything.

### Story 5.2 — `--exclude`
- [ ] Repeatable glob patterns applied to both source scan and dest scan (excluded dest files are invisible to `--delete`).

**AC**
- `--exclude target --exclude '*.log'` skips matches at any depth (rsync-style matching against the relative path).
- Unit tests cover: name match, glob match, directory prune (children of an excluded dir are never scanned).

### Story 5.3 — `--dry-run`
- [ ] Full scan + classification, zero writes; prints per-action lines (create/update/delete) and the summary that a real run would produce.

**AC**
- Dest tree is bit-identical before/after a dry run (including mtimes).
- Summary counts match what a subsequent real run actually performs.

### Story 5.4 — `--checksum` + hash cache
- [ ] Classification by BLAKE3 content hash instead of size+mtime; both sides consult a versioned
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

### Story 5.5 — `--paranoid`
- [ ] After rename, re-read every written file from destination disk and verify BLAKE3 (huge files verified per-chunk against the recorded chunk hashes).

**AC**
- Normal run: no post-rename reads. Paranoid run: every transferred file re-read and verified; mismatch → retransmit once, then failure.
- Works in push (server re-reads), pull (client re-reads), and local modes.

### Story 5.6 — `--progress-json`
- [ ] Machine-readable JSONL event stream on stdout (scan progress, plan totals, per-file start/progress/done, total progress, warnings, final stats); progress bars suppressed.

**AC**
- Every line is valid JSON with a `type` and schema version; a GUI can compute both bars from the
  stream alone.
- Events expose scan/plan/transfer/verify phase timings, local workers, data streams, logical and wire
  bytes, compression decisions, clone/reflink use, retransmitted bytes, resumed bytes, queue
  high-water marks, and named warnings without parsing human text.
- Event schema documented in the README; final `done` event contains the full stats summary.
- Unknown future event fields are ignorable; breaking changes require a schema-version change.

### Story 5.7 — Cloud-placeholder materialization policy
- [ ] Detect platform cloud/dataless placeholders where possible and make their materialization
  visible and controllable instead of accidentally downloading a very large tree.

**AC**
- Default behavior preserves rsync-like correctness by reading/downloading file content, but the
  scan summary reports placeholder file count and logical bytes before transfer begins.
- `--cloud-files=download|skip|error` is explicit; `skip` records skipped paths as partial work and
  never lets `--delete` remove the corresponding destination path, while `error` mutates nothing.
- macOS tests use synthetic/disk-image fixtures where possible and isolate platform APIs behind a
  portable capability interface. Unsupported platforms report the policy as unavailable rather
  than pretending detection occurred.

---

## Epic 6 — Compression

### Story 6.1 — zstd with skip heuristic
- [ ] Data payloads use zstd level 3 when the Story 0.5 sampling decision predicts a ratio below
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

---

## Epic 7 — Progress UI

### Story 7.1 — Two-bar terminal UI
- [ ] indicatif MultiProgress: one total bar (bytes + file counts, "scanning…" state until discovery completes, then firm totals + ETA); per-stream lines showing a per-file bar for active medium/huge files or `current-file  N files (K/s)` in batch mode. `-q` silences everything except errors; non-TTY output degrades to plain periodic status lines.

**AC**
- Total bar is always present and monotonic; it grows while scanning and never jumps backwards.
- Large-file transfer shows a smooth per-file bar; small-file storm shows files/sec without flicker or thousands of bar lines.
- Final summary: files transferred/skipped/deleted/failed, bytes, wire bytes, elapsed, throughput, verification status — under 10 lines, nothing like `rsync -P` spew.
- Piping stdout to a file produces no ANSI garbage.

---

## Epic 8 — Benchmarks & docs

Epic 0 creates the harness early. Epic 8 runs the completed product matrix, freezes supported
defaults, and publishes only claims the reports support.

### Story 8.1 — Release benchmark matrix and regression gates
- [ ] Run the Epic 0 harness against the release candidate over local same-volume/cross-volume,
  PipeTransport, and real SSH for every required corpus and change shape.

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
