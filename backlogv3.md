# xsync backlog v3 — from capable engine to shippable rsync alternative

Companion to [backlogv2.md](backlogv2.md) (protocol v2 and the browse surface),
[DEPLOYMENT.md](DEPLOYMENT.md) (CI, signing, packaging), and
[TUNING-TASKS.md](TUNING-TASKS.md) (performance experiments). This is the product-level
queue: it says what should happen next across those tracks, which work blocks a release,
and what evidence is required before a research idea becomes a feature.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done · `[—]` closed without
shipping. Priority legend: **P0** release blocker · **P1** daily-driver release · **P2**
parity or high-value follow-up · **R** measurement-gated research.

---

## Audited state — 2026-08-29

This snapshot is based on the working tree, not only on roadmap status boxes.

| Area | What exists now | Important qualification |
|---|---|---|
| Engine | Local copy, SSH push and pull, delete-after, dry-run, BLAKE3 classification, adaptive zstd, atomic staging, large-file chunk resume, APFS clone and Linux reflink paths | No remote-to-remote; whole-file transfer only |
| Correctness | Source-stability retries, destination containment, symlink refusal, path-collision probing, bounded/fail-closed frames, independent benchmark oracle | Default small/medium remote verification still hashes bytes only after receipt; sparse content is still materialized densely |
| Product surface | Named TOML jobs, ordered include/exclude files, `.xsyncignore`, JSON progress and failure logs, remote bootstrap, completions/man-page generation | Native remote include rules and remote ignore files are refused or unavailable; human output, cancellation, locking, timeouts and safe-delete controls are incomplete |
| Protocol/ecosystem | Frozen v1 sync protocol; negotiated v2 browse/mutate/fetch/publish library surface; cross-language v2 vectors | Browse is not a first-class `xs` CLI workflow; sync-path v2 and skew testing are incomplete |
| Delivery | Multi-platform CI and release workflows, reproducible packaging scripts, checksum/provenance flow, install script, Homebrew/Scoop/Linux-package scaffolds | No public release or end-to-end package-channel install is evidenced; macOS and Windows signing remain deferred |
| Quality | `cargo fmt --all -- --check`, `cargo test --workspace --all-targets`, and strict workspace Clippy are green; 319 tests passed and 2 opt-in stress/benchmark tests were ignored | Failure injection, cross-version binaries, cross-OS pairs and sustained fuzzing are not yet release gates |
| Performance | Strong large-file, no-op, compression and clone results; small-file batching fixed a 25.5x SSH regression | Local many-small-file initial copy is still about 2x slower than rsync in the retained measurement; multi-stream setup and capability bugs remain |

The documentation is not yet a reliable source of truth. For example, the README says both
that there is no packaging and that jobs/configuration do not exist, while the working tree
contains packaging, release automation and `config.rs`. `DEPLOYMENT.md` still opens by saying
there is no CI. Story V3.32 makes eliminating this drift a release requirement.

## Product target and release gates

"Comparable to rsync" does **not** mean accepting every historical rsync flag. It means a
user can select xsync for ordinary local and SSH synchronization without risking a weaker
integrity, recovery or automation contract. Unsupported semantics must be rejected before
mutation and documented in the compatibility table.

### Gate A — shippable technical preview

- [ ] Default end-to-end integrity is real for every size class; `--paranoid` is additional
      readback, not the only trustworthy mode.
- [ ] Sparse sources either preserve holes or are refused before transfer when the dense
      result cannot fit.
- [ ] Interrupt, destination locking, timeout and delete guardrails have deterministic exit
      codes and recovery tests.
- [ ] Crash, disconnect, ENOSPC, permission failure and source mutation tests assert the
      destination state, not only the error string.
- [ ] A versioned artifact installs and uninstalls on every Tier 1 platform through the
      documented path, with checksum/provenance verification.
- [ ] The support, security and compatibility matrices match the released binary.

### Gate B — dependable daily driver

- [ ] The common selection and safety surface is present: `-a`, `-v`, `--itemize-changes`,
      `--stats`, `--files-from`, `--max-delete`, `--bwlimit`, `--timeout`, `--update`,
      `--ignore-existing`, `--max-size`, `--min-size`, and recoverable deletion.
- [ ] Native push and pull have the same filter semantics as local sync.
- [ ] `xs doctor`, config validation, state inspection and actionable retry output cover the
      failures an operator otherwise discovers mid-run.
- [ ] No supported regression-tier workload is below 0.9x its paired rsync baseline, unless
      the release notes name the workload and the user can select a safe fallback.

### Gate C — credible rsync replacement

- [ ] Sparse files, hardlinks, xattrs/ACLs, ownership policy and common archive semantics are
      either preserved or explicitly excluded by the declared product scope.
- [ ] Delta transfer has a measured go/no-go decision and, if selected, is corruption- and
      interruption-tested against rsync's algorithm.
- [ ] Cross-version and cross-platform matrices have no undefined cells.
- [ ] Every supported rsync-compatibility flag is implemented or fails before mutation with
      a specific alternative.

## Priority board

| Order | Priority | Stories | Outcome |
|---:|---|---|---|
| 1 | **P0** | V3.20–V3.23, V3.11–V3.13 | Integrity, fault recovery, bounded waiting and destructive-operation safety |
| 2 | **P0** | V3.31–V3.32 | One honest, installable preview rather than more unverified packaging surface |
| 3 | **P1** | V3.14–V3.16, V3.24–V3.30 | Daily-driver UX, common rsync semantics, metadata policy and compatibility |
| 4 | **P1** | V3.18, V3.33–V3.36, V3.40 | Close known latency regressions and prevent them from returning |
| 5 | **P2** | V3.17, V3.41–V3.44 | Configuration, scheduling, cleanup and retry ergonomics |
| 6 | **R** | V3.4–V3.8, V3.37–V3.39 | Structural speedups that ship only after their decision gates pass |

---

## Part A — Correctness gaps that block daily use

These are not polish. Each is a reason not to trust the tool with real data.

### Story V3.1 — Destination path collisions silently lose files
- [x] **Fixed.** Two source paths that are distinct on the source can be the same path
  on the destination. xsync published both and kept whichever wrote last.

**Resolution.** `crates/xsync-core/src/pathsem.rs` probes the destination for case- and
normalization-insensitivity by creating probe names and observing whether they collide,
falling back to the nearest existing ancestor when the destination is not yet created.
The planner groups destination paths by a folding key and refuses the run before any
write, naming the collision. `--on-path-collision=skip` omits colliding paths instead and
exits with the partial-failure code. Wired into both the local engine and the pull client,
whose destination is also local. Covered by unit tests for the folding and policy, and by
an integration test that self-skips where the source volume cannot construct the case
(on APFS the *source* folds the names too, so a true end-to-end test needs a
case-sensitive source volume).

**Measured, not hypothesised.** A Linux source containing four files:

```
café.txt   (NFC, U+00E9)      Readme.md
café.txt   (NFD, e + U+0301)  readme.md
```

pulled to macOS/APFS produces **two files**, with **exit code 0 and no warning**.
`Readme.md` survives holding `readme.md`'s content. Half the data is gone and nothing
says so.

Two distinct causes, both properties of the *destination*:

- **Case-insensitivity.** APFS and NTFS are case-insensitive by default; ext4 is not.
- **Unicode normalization-insensitivity.** APFS treats NFC and NFD forms of the same
  name as one file. (APFS is normalization-*preserving* — a round-trip of a single NFC
  name is byte-exact, which was verified — so the hazard is collision, not mangling.)

**AC**
- The planner detects, before any write, that two source entries map to one destination
  path under the destination's actual case and normalization behaviour.
- A collision is a **failure**, not a warning: the run reports the colliding paths and
  exits non-zero, because silently keeping one is the worst option available.
- Destination behaviour is **probed**, not assumed from the platform — create two probe
  names and observe. macOS volumes can be case-sensitive and Linux can host NTFS.
- An opt-in flag chooses a documented resolution (skip both, or suffix) for users who
  knowingly want to proceed.
- Covered by an integration test on a case-insensitive destination.

### Story V3.2 — Sparse files still write their apparent size
- [x] **Warning implemented.** The transfer itself is still dense — that is
  TUNING-TASKS T2 — but xsync no longer starts the work silently.

**Resolution.** `crates/xsync-core/src/sparse.rs` inspects every planned file at or above
1 MiB, comparing `st_blocks * 512` against the apparent size, and reports any file
occupying less than half its length. Each is named with both sizes and its amplification,
followed by a total of the holes that will be materialized. Wired into the local engine
and the push client, both of which have a local source; `--dry-run` reports the same, so
the cost is visible before committing. Verified against the real Docker VM disk:
3,996 GB apparent, 140 GB allocated, **28.6x**, matching the independent `SEEK_HOLE`
measurement exactly. Windows reports nothing rather than guessing, since std does not
wrap `GetCompressedFileSize`.

A 130 GB sparse VM image is written as 3.7 TB. It does not degrade; it fills the disk
and fails after hours. Until T2 lands, xsync should **refuse or warn loudly** on a
source file whose allocated size is far below its apparent size, rather than starting
work it cannot finish.

**AC**
- A source file with an allocated:apparent ratio below a threshold produces a clear
  up-front warning naming the file and both sizes.
- `--dry-run` reports the true byte cost, so the failure is discoverable before the run.

### Story V3.3 — Metadata xsync silently drops
- [x] v1 does not preserve hardlinks, xattrs, ACLs, ownership, or resource forks.

Documented in the README, but the run itself says nothing. A user copying a home
directory or a Time Machine-adjacent tree gets a destination that looks complete and
is not.

**AC**
- The scanner counts entries carrying metadata that will not survive (hardlink count > 1,
  non-empty xattrs, non-trivial ACLs) and reports the totals in the summary and in the
  `done` event.
- `--dry-run` reports the same, so it is visible before committing.
- Silence is reserved for the case where nothing is being dropped.

**Done.** `sparse.rs` became a single preflight pass reporting both sparseness and dropped
metadata. Hardlinks, extended attributes and foreign ownership are counted and reported in
the summary and in the `finished` event (`dropped_hardlinked_files`,
`dropped_hardlink_extra_bytes`, `dropped_xattr_entries`, `foreign_owner_entries`).

Three things the AC assumed that turned out not to hold, each resolved deliberately rather
than quietly:

1. **ACLs are not counted.** Detecting them needs `libacl` on Linux and `acl(3)` on macOS,
   neither a current dependency. Documented in the README as a gap rather than omitted.
2. **`com.apple.provenance` is on nearly every macOS file.** Counting every non-empty xattr
   list made the warning fire on a tree containing one plain text file, which would have
   destroyed the "silence means nothing is dropped" contract. Kernel-maintained attributes
   are filtered out.
3. **`--dry-run` cannot answer the ownership question.** The only non-guessing way to learn
   who will own the copies is to create a file where they will go, and a dry run must not
   write to the destination. Answering from a scratch directory was tried and reported
   every file as foreign-owned purely because the temp volume had a different group. A dry
   run now leaves ownership unchecked and says so. Hardlink and xattr counts are identical
   between the two modes.

**Cost, measured** (congress-100k, 109,615 files, local APFS, paired reps, same binary
behind a temporary toggle): no measurable difference at 10k files; **+11.7%** at 100k,
split roughly evenly between the added `stat` and the added `listxattr`.

**Follow-up worth doing.** The `stat` is redundant: the scanner already holds that metadata.
Carrying `uid`, `gid` and `nlink` on `SourceFingerprint` was implemented and then reverted,
because plan entries reach the preflight reconstructed from the index encoding, which does
not carry them — and that encoding is shared with the frozen v1 wire format. Extending it
would remove about half the cost of this feature and is a protocol-versioning question, not
a preflight one.

---

## Part B — Research

### Story V3.4 — Filesystem-level send as a fast path
- [ ] Where both ends are the same filesystem type, per-file work can be bypassed
  entirely: `zfs send`/`recv`, `btrfs send`/`receive`.

This is the only idea on the table that changes the asymptotics rather than the
constant. The materialization floor measured in T1.4 was 0.106 s for 11,280 files;
a filesystem stream does not materialize files at all.

**AC**
- A measurement spike first, on `freya` (ZFS): `zfs send` of a snapshot against xsync
  and `rsync -a` for the same subtree.
- **Decision gate:** below 3x over xsync's own path, do not build it. A capability this
  narrow — both ends one filesystem type, source must be a snapshot — must earn its
  place decisively.
- If built, it is an explicitly selected fast path, never an automatic substitution:
  the guarantees differ (no per-file verification, whole-snapshot granularity).

### Story V3.5 — Content-defined chunking and cross-file dedup
- [ ] Tracked as TUNING-TASKS T5; the measurement is still unrun.

cb7 holds ~4.0 GB of redundancy among files over 50 MB, with `libreader_lib.rlib`
confirmed byte-identical across build profiles. rsync cannot exploit this — its delta
compares a file only against the same path.

### Story V3.6 — Bandwidth shaping and link-aware strategy
- [ ] The host-to-host measurements showed xsync **2.47x faster** than rsync on a
  5.8 MB/s link and **0.84x** on a 39 MB/s one, purely from compression paying off
  differently.

**AC**
- A `--bwlimit` equivalent, which rsync has and xsync does not.
- Investigate whether the compression decision should consult *measured* link
  throughput rather than only payload compressibility. The crossover exists and has
  been measured; the engine does not currently know which side of it it is on.

### Story V3.7 — Verification without transfer
- [ ] A mode that answers "are these two trees identical?" without writing anything.

Distinct from `--dry-run`, which compares metadata. This would compare content, reusing
the hash cache, and is the natural way to audit a backup. It is also the honest way to
demonstrate the integrity claim to a sceptical user.

### Story V3.8 — Multi-destination fan-out
- [ ] Sync one source to N destinations in one pass, reading and hashing once.

The scan, plan, and read are shared; only publication differs. For anyone mirroring to
a NAS and an external disk this is strictly better than N sequential runs, and the
architecture already separates source reading from sink publication.

---

## Part C — Daily-driver polish

The gap between "correct" and "the tool you type without thinking".

### Story V3.9 — Named jobs and a config file
- [x] `xs backup` should run a saved source, destination, and flag set.

The single largest ergonomic gap. Nobody retypes an exclude list and a destination path
daily; they write a shell alias, and then the tool's own `--dry-run` and logging never
see the real configuration.

**AC**
- A documented config format and search path, with named jobs.
- CLI flags override config; precedence is defined and tested.
- A malformed config fails at startup with a precise error, never partial application.
- `xs --dry-run <job>` works, so a saved job can be inspected before it runs.

**Done.** TOML config in `crates/xsync/src/config.rs`, with `--job NAME`, a bare positional
job name, `--config FILE`, and `--list-jobs`. Search path: `--config` > `$XSYNC_CONFIG` >
`$XDG_CONFIG_HOME/xsync/config.toml` > `~/.config/xsync/config.toml` (`%APPDATA%` on
Windows). XDG is used on macOS too — a config file gets copied between a laptop and a
server, and one path everywhere beats platform purity.

**Precedence** is flag > job > built-in default, resolved through clap's `ValueSource`
rather than by comparing values. This is the part that would have been quietly wrong
otherwise: `--transport auto` is indistinguishable from an untouched default by value, so a
value-based merge would let the job override a flag the user explicitly typed. `main` now
parses via `ArgMatches` and `from_arg_matches` for exactly this reason. Tested, including
that case specifically. `--exclude` on the command line replaces rather than extends the
job's list.

**Never partially applied**: the whole file is parsed and every job validated — endpoints
parsed and paired, enums checked, numeric ranges checked — before any value reaches the
run. `deny_unknown_fields` makes `excludes = [...]` a startup error with line, column and
the list of valid keys, rather than a backup that silently copies files the user believed
were excluded.

**Two judgement calls worth recording:**

1. **A bare `xs backup` that is both a job and an existing directory is refused, not
   guessed.** Either resolution could copy the wrong tree. The error names both ways to
   disambiguate. This turned up immediately in manual testing, where the fixture had a
   `backup/` directory.
2. **A boolean a job enables cannot be disabled from the command line**, because there is
   no `--no-delete`. A job with `delete = true` always deletes. Documented in the README
   rather than papered over; the natural fix belongs with V3.11, which is already about
   making `--delete` survivable.

One consequence to be aware of: `xs <one-positional>` is no longer a clap parse error, so
the "expected SRC and DEST" message is now produced by job resolution instead. It names the
config files it looked at, and lists the jobs that do exist when there is a config.

### Story V3.10 — Exclude ergonomics
- [x] `--exclude <GLOB>` exists. `--exclude-from <FILE>` and an ignore-file convention
  do not.

**AC**
- `--exclude-from`, and a documented per-tree ignore file.
- Include/exclude precedence documented with worked examples — this is the single most
  confusing part of rsync, and is worth getting right rather than copying.
- `--dry-run` shows which rule excluded a path when asked.

**Done.** New `crates/xsync-core/src/filter.rs`: ordered rules, first-match-wins, with the
origin of every rule tracked so a decision can be explained. `--include`, `--exclude-from`,
`--include-from`, `--no-ignore-file` and `--explain-filter` on the CLI; `.xsyncignore` as the
per-tree convention.

**Order is recovered from `ArgMatches::indices_of`.** clap groups repeated occurrences by
flag, so `--include a --exclude b` and `--exclude b --include a` arrive identically —
first-match-wins would have been meaningless. A `--exclude-from` file expands where its flag
appeared, not at the end.

**Three deliberate departures from rsync**, each tested:

1. **No `--include '*/'`.** A directory is walked whenever an include rule could match
   beneath it, computed from the rules. `rsync -a --include 'docs/**' --exclude '*'`
   transfers nothing and reports success; the xsync equivalent transfers `docs/`.
   Over-descending costs a readdir, under-descending loses files silently.
2. **An explicit rule about a path beats an inherited one**, which is what makes "exclude
   the tree, keep one file in it" expressible.
3. **`.xsyncignore` is weaker than the command line.**

**Two bugs found by testing the thing I had just built**, both worth recording because the
first version looked like it worked:

- I first delegated `.xsyncignore` to `ignore`'s `add_custom_ignore_filename`. It prunes
  during the walk, which made the ignore file *stronger* than every command-line rule — the
  exact opposite of what I had documented one commit earlier — and made those paths invisible
  to `--explain-filter`, because a pruned path is never seen again. Replaced with a
  second-tier `IgnoreLayer` that the scanner populates per directory (the walker calls
  `filter_entry` on a directory before yielding its children, which is what makes this
  well-defined under a parallel walk).
- The ignore-tier ancestor walk stopped at the first directory without an ignore file, so a
  root-level `*.log` matched `build.log` but not `logs/build.log`. Fixed and pinned by a test.

**The remote story, stated rather than fudged.** The v1 wire carries a flat exclude list.
`--exclude-from` patterns are now folded into that list — before this they were silently
dropped on every remote transfer, since the wire got `cli.exclude` and not the filter.
`--include` is *refused* for remote transfers: an include rule's meaning is its position
among the excludes, and sending the excludes alone would transfer a larger set than asked
for, silently. `.xsyncignore` warns rather than fails when the source is remote, because an
unseen ignore file cannot widen the transfer beyond the explicit rules.

**Follow-up.** Carrying the whole ruleset needs a `CAP_FILTER_RULES` capability bit, the
`"+ pattern"` / `"- pattern"` wire encoding (already implemented and round-trip tested in
`filter::encode`/`filter::decode`), and a server-side `FilterSet`. The protocol reserves
un-masked capability bits for exactly this, so it is not a version bump.

### Story V3.11 — Make `--delete` survivable
- [ ] **P0.** `--delete` permanently removes files with no undo, maximum or confirmation.

**AC**
- A `--backup`/trash mode moving deletions aside instead of unlinking.
- A summary of what will be deleted **before** deleting, and a confirmation for
  interactive runs above a threshold count or fraction.
- A refusal when the deletion set exceeds a suspicious share of the destination —
  the classic "source failed to mount, mirror wipes the backup" accident.
- `--max-delete=N` is enforced in the planner before the first deletion. Non-interactive
  runs never prompt and require an explicit policy when the safety threshold is exceeded.
- Backup/trash paths are on the destination filesystem when possible so protection does not
  turn each delete into a full cross-device copy. Retention and cleanup are documented.
- Delete summaries and backup locations are included in JSON output and retry/recovery tests.

### Story V3.12 — Interrupt cleanly
- [ ] **P0.** There is no `SIGINT` handling. Ctrl-C leaves whatever state it lands in.

**AC**
- `SIGINT`/`SIGTERM` stop dispatch, let in-flight files finish or abandon their staging
  files, print a summary, and exit with a distinct code.
- Staging files (`.xsync.tmp.*`) are removed or documented as resumable.
- A second Ctrl-C exits immediately.
- No delete phase begins after cancellation, and remote children/sessions are reaped within a
  bounded time.
- Exit status and JSON distinguish user cancellation from transport or integrity failure.

### Story V3.13 — Concurrent-run safety
- [ ] **P0.** Two xsync runs against one destination will interleave writes. `fslock` is already
  a dependency, used only by the resume journal.

**AC**
- A destination lock, with a clear message naming the holding process.
- The lock carries run ID, pid, host, start time, endpoints and version; local, push-sink and
  pull-destination paths all acquire it before mutation.
- Stale locks have a separate, explicit break operation. A generic `--force` never overrides
  a lock whose owner is still provably active.
- Overlapping roots are handled deliberately: `/backup` and `/backup/photos` cannot run as if
  they were unrelated destinations.

### Story V3.14 — Output a human wants to read
- [ ] **P1.** `--progress-json` is excellent. The human-facing output is not yet its equal.

**AC**
- A final summary reporting files transferred, skipped, deleted, failed, bytes moved
  versus bytes on the wire, and elapsed time — in under ten lines.
- An `--itemize`/`-v` mode naming what changed and why, which is how a user builds trust
  before enabling `--delete`.
- `--stats` for the detail, off by default.
- **Remove the `[xsync server] ...` diagnostics that currently print to stderr on a
  normal remote run.** They are debug output in the default path.
- Colour and progress suppressed when not a TTY.

### Story V3.15 — rsync muscle memory
- [ ] **P1.** Users arrive with rsync flags in their fingers.

**AC**
- Accept `-a`/`--archive` as an explicit no-op alias with a note, since xsync's defaults
  are already archive-like — silently rejecting it is a bad first impression.
- Support or explicitly reject, with a helpful message, the flags people will reach for:
  `-v`, `--progress`, `-z`, `--partial`, `--update`, `--ignore-existing`, `--bwlimit`,
  `--max-size`, `--min-size`.
- A documented rsync-to-xsync flag table in the README.

### Story V3.16 — `xs doctor`
- [ ] **P1.** One command that explains the environment before the user hits it as a surprise.

**AC**
- Reports: reflink support at the destination (the probe already exists), remote xsync
  presence and version, protocol compatibility, SSH reachability, destination
  case/normalization behaviour (from V3.1), free space against the estimated need, and
  which metadata classes will be dropped (from V3.3).
- Exits non-zero when it finds something that would fail the real run.

### Story V3.17 — Scheduling that is not cron-by-hand
- [ ] **P2.** DEPLOYMENT D6 covers the daemon. This is the smaller, nearer thing: making a
  scheduled xsync run pleasant.

**AC**
- Documented launchd/systemd-timer examples for a named job from V3.9.
- Machine-readable exit codes distinguishing "nothing to do", "transferred", "partial
  failure", and "refused", so a scheduler can act on them.
- Log rotation guidance for `--log-json`.

---

### Story V3.18 — `--streams` correctness and the small-file path

- [~] **P1.** Three defects found while investigating the congress `--streams 8` regression
  (2026-08-28). Two fixed, one open. Full evidence in `BENCHMARKv2.md`.

**Fixed.** Small files now go through `send_small_files_batched`, a single shared
implementation called by both `run_client_push` and the multi-stream control session
(the duplication was the cause of the divergence). Measured on cb7 at `--streams 8`:
149.3 s -> 55.3 s, **2.70x**. Single-file sources fixed by passing
`source_reader_root` to `run_data_thread`; verified by SHA-256 on an 885 MB striped
transfer.

**Still open:** `--streams 16` fails with `Broken pipe` (N+1 SSH connections exceed
OpenSSH's default `MaxStartups`), connections are still established sequentially
(~1.3 s fixed cost at 4-8 streams), and the multi-stream control session still
negotiates `capabilities=0x0`, so its small-file traffic is never compressed.

**What is actually wrong.** `--streams N > 1` dispatches to
`sync_push_server_streams` (server.rs:5206–5958), an implementation that predates
the batching and pipelining work and never received it. Every pipelining call site
(`MAX_PIPELINED_FRAMES` / `drain_acks`) is in the single-stream paths at 2974–4067.

**AC**

- **Small files stop costing a round trip each.** The multi-stream control session
  reuses the batched sender `run_client_push` already has (server.rs:3633–3660,
  coalescing to `BATCH_TARGET_SIZE` with pipelined acks), leaving the data threads
  to carry large-file ranges. Measured target: congress-1k at 1076 files should go
  from 4.42 s back to parity with single-stream's 0.35 s.
- **Single-file sources work.** `run_data_thread` receives `source_reader_root`,
  not the raw `source_path`, so a file source stops producing
  `<file>/<file>`. Today every `--streams N > 1` transfer of one file fails.
- **`--streams 16` stops failing with `Broken pipe`.** xsync opens N+1 SSH
  connections; 17 exceeds OpenSSH's default `MaxStartups 10:30:100`. Either cap
  the concurrent connections, establish them with retry/backoff, or refuse the
  request up front with a message naming the cause — the current failure is opaque
  and arrives after several sessions have already succeeded.
- Connections are established concurrently rather than in a sequential
  `spawn_server_child` loop. Measured fixed cost today: 0.19 s at 1 stream,
  ~1.3 s at 4–8.

**Do not "fix" the stream count.** On large files the flag already works: with
setup subtracted, `--streams 4` transfers at 106 MB/s against a measured 106 MB/s
wire ceiling. Four streams is enough to saturate gigabit, and eight is worse than
four. The prize on this link is ~1.2x and it is already claimed; the value of a
higher stream count is on faster links, where single-stream xsync's 83 MB/s
pipeline would leave much more of the wire idle.

### Story V3.19 — The serial bottleneck that caps local scaling at 12 workers

- [—] **Withdrawn 2026-08-29: the premise was a measurement artifact.** The
  12-worker plateau appeared only with warm caches. Re-measured cold on
  congress-1m with `drop_caches` before every rep, scaling improves monotonically
  to 32 workers in all three device configurations (67.3 -> 53.6 s internal->USB,
  104.2 -> 70.1 s USB->internal, 111.8 -> 77.9 s ZFS->USB). With a warm cache
  there is no I/O to wait on, so the run is CPU- and lock-bound and extra threads
  have nothing to hide; cold, each file carries real device latency. The `tar`
  comparison that motivated this was warm too, and is equally suspect.
  `default_local_workers` (one per logical core) is correct as it stands.
  Evidence in `BENCHMARKv2.md`.

The original text follows, for the record.

At 557 MB / 1.95 s the run is bound by per-file work, not bandwidth — NVMe is
nowhere near saturated, and 20 of 32 worker threads contribute nothing.

**AC**

- The serial component is identified and named, with a measurement rather than a
  hypothesis. Candidates not yet distinguished: the `ensured_directories` mutex
  every worker takes through `Sink::create_parent`; ext4 directory-lock
  contention on file creation; the local planner's round trip of every entry
  through the index encoding (which is also why the preflight cannot reuse the
  scan's `stat` — see V3.3).
- Scaling continues past 12 workers, or the default is lowered to match reality
  and `default_local_workers` documents why, the way `MACOS_WORKER_CAP` does.
- The `tar` comparison is re-run; beating a single-threaded pipe on a 32-thread
  host is the bar.

**Already fixed here:** the V3.3 preflight ran serially on one thread and cost
16.4% of the run. `sparse::inspect_with_workers` now chunks it across the worker
pool, taking that to ~1%, with a test asserting the parallel result is identical
to the serial one for hardlink groups spanning chunk boundaries.

---

## Part D — P0 correctness and recoverability

These stories define Gate A. A preview release is not honest while any is open.

### Story V3.20 — Make end-to-end content integrity the default
- [ ] **P0.** Small and medium remote files are currently "verified" against a digest
  computed from the received buffer. That detects a bad disk write but cannot detect data
  altered before or during receipt. Large-file sender digests only become a full-file check
  under `--paranoid`.

**AC**
- The sender's BLAKE3 digest is carried independently for every complete-file transfer and
  compared before publication. Chunked files also compare the sender's full-file digest
  before the final rename.
- A test corrupts payload bytes after the sender computes the digest and proves the receiver
  rejects them, removes or retains only resumable staging state, and preserves an existing
  destination.
- Local, push, pull, one-stream and multi-stream paths share the same verification contract.
- `--paranoid` means re-read the published destination from storage. It is no longer required
  for ordinary end-to-end integrity.
- The sync protocol change is capability/version negotiated. An older peer never receives a
  frame it cannot decode; the CLI names the reduced guarantee and requires an explicit policy
  to use it rather than silently weakening the default.
- Protocol vectors, compatibility matrix, JSON events and README all name the selected
  integrity level.

### Story V3.21 — Preserve sparse files, with a safe refusal until then
- [ ] **P0.** V3.2 made amplification visible; it did not make the transfer safe. Complete
  [TUNING-TASKS Epic T2](TUNING-TASKS.md#epic-t2--sparse-aware-transfer-deferred).

**AC**
- APFS/ext4/btrfs/XFS sources enumerate data extents with `SEEK_DATA`/`SEEK_HOLE`; Windows
  uses `FSCTL_QUERY_ALLOCATED_RANGES`. Unsupported filesystems fall back only through an
  explicit, observable policy.
- The wire represents holes as ranges without sending zero bytes. The sink punches/seeks
  holes and verifies logical size, content digest and allocated-byte expectations.
- Large extents and thousands of small extents stay bounded in memory and remain resumable.
- Paired correctness/performance coverage uses `rsync -aS` and a non-compressing filesystem;
  ZFS compression is not accepted as proof that holes were reproduced.
- Until this ships, xsync refuses a sparse transfer when estimated dense bytes exceed free
  space or a configurable amplification ceiling. `--allow-dense-sparse` is an explicit
  override and is present in JSON output.

### Story V3.22 — Turn failure modes into a release test matrix
- [ ] **P0.** Happy-path coverage is strong; product trust comes from proving what remains
  after the unhappy path.

**AC**
- Deterministic tests inject: client kill, server kill, SSH disconnect, short read/write,
  corrupt frame, ENOSPC during stage and publish, read-only destination, permission denial,
  disappearing source, source mutation, destination mutation, and failure during metadata
  restoration.
- Every cell asserts final destination content, staging/journal state, delete suppression,
  emitted failure kind and exit code. "Returned an error" alone is insufficient.
- Rerunning after every recoverable failure converges to the independent manifest without
  retransmitting journal-verified large-file ranges.
- A failed transfer never performs delete-after and never exposes a truncated final file.
- At least one push and one pull fault cell run as real child processes rather than only
  in-memory transports.
- The matrix is documented and runs in CI at a bounded tier; slower power-loss/fsync tests
  run on a scheduled or release tier.

### Story V3.23 — Bound waiting and retry only safe failures
- [ ] **P0.** SSH setup, frame reads and stalled peers can currently wait indefinitely.

**AC**
- Add separately observable connect, idle-I/O and optional whole-run timeouts. Defaults are
  conservative and documented; zero disables only when the user asks.
- Keepalive distinguishes a slow active transfer from a dead peer. Timeout errors name the
  phase, host and last progress time.
- Retry/backoff is limited to classified transient transport failures. Authentication,
  host-key, protocol, path, permission and integrity failures are never retried.
- A retry reuses safe resume state and never repeats an already-acknowledged destructive
  mutation.
- `SIGINT` interrupts timeout waits promptly and composes with V3.12's two-stage cancellation.
- Fake transport tests pin attempt counts, backoff bounds and total elapsed-time bounds.

---

## Part E — P1 shippable product and rsync workflow parity

### Story V3.24 — Destination capacity and topology preflight
- [ ] **P1.** Fail before a long run when the destination obviously cannot accept the plan.

**AC**
- Estimate logical writes, allocated writes, staging headroom, backup/trash headroom and
  metadata overhead. Keep each quantity distinct rather than publishing one misleading
  "bytes needed" number.
- Probe free space, read-only state, path semantics and reflink support at the actual
  destination or nearest existing ancestor. Remote results are requested from the remote,
  never inferred from the client platform.
- `--dry-run` and `xs doctor` show the estimate and uncertainty. A known shortfall refuses
  before mutation; an unmeasurable value is labelled unknown.
- Source-inside-destination, destination-inside-source, missing mount and unexpectedly empty
  source protections are covered, including the "backup disk was not mounted" case.

### Story V3.25 — Preserve metadata by declared profiles
- [ ] **P1/P2.** Warnings are the right interim behavior, but an rsync alternative needs a
  preservation story that is clearer than one overloaded archive switch.

**AC**
- Define profiles: `portable` (current cross-platform subset), `archive` (hardlinks, xattrs,
  ACLs and timestamps), and `system` (ownership, IDs, devices/specials where privileged).
- Hardlinks use a bounded inode/file-ID map and retain link topology across local, push and
  pull. Repeated content is not silently materialized as independent files.
- xattrs include macOS resource forks; ACLs round-trip on macOS and Linux with an explicit
  cross-platform translation/refusal policy.
- Ownership supports `--numeric-ids` and an explicit mapping policy. Lack of privilege is a
  preflight result, not a late stream of `chown` failures.
- Unsupported metadata is counted before mutation and represented in JSON, including ACLs,
  which the current preflight cannot inspect.
- A metadata corpus and oracle compare xsync with `rsync -aHAX` on same-platform routes and
  verify the declared portable subset on cross-platform routes.

### Story V3.26 — Implement the common rsync selection semantics
- [ ] **P1.** Group the cheap classifier options, but specify their interactions before
  adding flags one by one.

**Scope**
- `-a`/`--archive`, `-v`, `--update`, `--ignore-existing`, `--existing`, `--size-only`,
  `--ignore-times`, `--max-size`, `--min-size`, `--relative` and `--mkpath`.

**AC**
- A compatibility table says exact, deliberately different, or unsupported for each flag.
- Combinations have one documented precedence model and table-driven classifier tests.
- A flag never becomes an accepted no-op unless its promised behavior is already the default
  and the help text says that explicitly.
- Local, push and pull behave identically or reject before opening the destination.
- Golden tests run representative commands against rsync and compare the selected path set
  and destination manifest, not human wording.

### Story V3.27 — Explicit path lists, list-only and native remote filter parity
- [ ] **P1.** Complete the scripted-selection workflows and remove the current local/remote
  semantic split.

**AC**
- Add `--files-from` with NUL-delimited input, stdin support and raw-byte Unix paths. Define
  roots, ordering, duplicates and missing entries before implementation.
- Add `--list-only` and a machine-readable plan/list mode. Reuse v2 browse primitives where
  appropriate without making a tree sync depend on one-page directory requests.
- Negotiate and send the complete ordered `FilterSet` (include/exclude and rule origin) to a
  native remote source. A remote `.xsyncignore` is evaluated by that source.
- An older peer either receives the existing safe exclude-only subset or the command refuses;
  includes are never approximated by transferring more data.
- Local, push and pull produce the same selected-path manifest for a shared conformance corpus.

### Story V3.28 — Windows as a real remote and topology clarity
- [ ] **P1/P2.** Windows initiator support exists; a stock Windows OpenSSH server must become
  a supported endpoint before claiming Tier 1 route parity.

**AC**
- Command construction, missing-binary detection, remote bootstrap, home expansion and path
  quoting work under stock `cmd.exe` and PowerShell without requiring Git Bash.
- Windows destination tests cover drive letters, UNC paths, reserved names, trailing dots,
  case collisions, long paths, symlinks/reparse points and non-UTF-8 limitations.
- Cross-OS push and pull run in CI or in a documented release lab, with the manifest oracle.
- Remote-to-remote receives an explicit product decision: direct relay, third-party copy, or
  documented refusal. If implemented, bytes need not flow through the controller unless the
  user chooses that mode.

### Story V3.29 — Cross-version policy and executable compatibility matrix
- [ ] **P1.** Source-level vectors are necessary but do not prove released binaries work
  across version skew.

**AC**
- State the supported client/server skew for sync v1, sync v2 and browse v2, including what
  security fixes can revoke compatibility.
- CI downloads or builds the oldest supported binary and exercises new-client/old-server and
  old-client/new-server push, pull and browse. Every cell is `works`, `degraded` with named
  guarantees, or `refused` with an exact class of error.
- Capability negotiation is recorded once before data frames; no feature is discovered by
  starting a mutation and catching a protocol error.
- Extend conformance vectors to the sync path, including content digests, filters, sparse
  extents and malformed boundary cases introduced by v3 work.

### Story V3.30 — Security and trust gate
- [ ] **P1.** Turn the existing fail-closed design and dependency checks into a published,
  continuously tested security posture.

**AC**
- Publish a threat model covering the local caller, SSH peer, server root, path traversal,
  symlink races, decompression bombs, frame limits, temp files, journals and bootstrap binary.
- Run protocol fuzzing continuously or on a schedule with retained corpus/crash artifacts;
  gate releases on zero known reproducible crashes.
- Test a malicious peer sending maximum counts, decompression expansion, duplicate IDs,
  invalid paths and messages in the wrong role without unbounded CPU, memory or disk use.
- Add `SECURITY.md`, a contact and response policy. Generate an SBOM and keep `cargo audit`,
  `cargo deny`, checksums and provenance in the release evidence.
- Any `unsafe` exception (currently the measured macOS `clonefile` wrapper) is isolated,
  documented and directly tested; the workspace-wide deny remains the default.

### Story V3.31 — Ship and rehearse the first public preview
- [ ] **P0.** Packaging scaffolds do not count as distribution until a clean machine installs
  a real tagged release from the channel a user will use.

**AC**
- Select one semantic version and freeze its support matrix, protocol compatibility, known
  limitations and changelog entry.
- Run the tag workflow, publish every Tier 1 archive, checksum set, provenance attestation,
  man page and completions, then install each artifact on a clean machine.
- Exercise install, `xs --version`, local sync, remote bootstrap, push, pull, upgrade and
  uninstall. The installed binary—not `cargo run`—must pass the smoke.
- Complete at least one package channel end to end (Homebrew is the smallest current gap),
  and label other manifests preview-only until their repositories accept a real package.
- Decide whether code signing blocks the preview. If deferred, test and document the exact
  browser-download versus package-manager behavior instead of saying merely "unsigned".
- Record the rehearsal as a release checklist artifact and repeat it for every tag.

### Story V3.32 — One source of truth for status and compatibility
- [ ] **P0.** Current documents contradict the code and each other.

**AC**
- Generate CLI flag, target, package and protocol tables from authoritative code/config where
  practical. Hand-written prose links to those tables instead of restating volatile facts.
- Reconcile README, MVP, DEPLOYMENT, backlog v2/v3 and TUNING status. Historical measurements
  remain historical and carry commit, host, cache state and date.
- A docs check catches placeholder URLs, stale version/test totals, broken local links and
  claims such as "no CI" when the workflow exists.
- README's first page says plainly: release status, install status, safe workloads, known
  integrity/sparse limitations and the shortest successful local/push/pull examples.
- Archive or label superseded plans. A new contributor can identify the active backlog
  without reconciling six documents by hand.

---

## Part F — Speed and latency research

Every story here starts with a benchmark or trace. A negative result closes the story and is
retained; it does not justify a speculative production subsystem.

### Story V3.33 — Establish latency budgets by phase and workload
- [ ] **P1 research prerequisite.** Replace a single wall-time number with a budget that
  identifies the first bottleneck worth fixing.

**AC**
- For local, push and pull, record connection/bootstrap, scan, preflight, plan, first-byte,
  data, metadata, delete and teardown times plus CPU, peak RSS, syscall counts and wire bytes.
- Cover: one tiny file, 10k/100k/1m tiny files, one large incompressible file, compressible
  data, no-op, 1% churn and high-RTT/bandwidth-shaped routes, warm and genuinely cold.
- Add user-facing latency metrics: time to first progress, time to first published file and
  cancellation latency, not only total throughput.
- The report names the top two contributors in each cell. Optimization work must cite the
  cell and budget component it intends to move.

### Story V3.34 — Reduce SSH setup latency and reuse connections safely
- [ ] **P1/R.** One SSH setup is visible for tiny transfers; multi-stream currently pays N+1
  setups and performs them sequentially.

**Experiment**
- Compare sequential versus concurrent setup, OpenSSH ControlMaster multiplexing, one
  long-lived v2 session carrying multiple sync requests, and a native SSH library spike.
- Measure 1/10/100 tiny sync jobs across LAN and 20/80/150 ms RTT, including authentication
  agent behavior and teardown.

**Decision gate / AC**
- Ship concurrent setup if it removes at least 50% of multi-stream setup time without
  triggering `MaxStartups`; use bounded concurrency plus jittered retry.
- Ship session reuse only if median repeated-job latency improves at least 2x and isolation is
  preserved: each job has a fresh root/policy context, cancellation and audit record.
- A native SSH stack must beat OpenSSH multiplexing materially and pass host-key, agent,
  ProxyJump, config-file and security review; otherwise keep OpenSSH.
- Never modify the user's SSH config automatically.

### Story V3.35 — Remove process-per-clone and choose native copy primitives
- [ ] **P1.** macOS now calls `clonefile(2)` directly; Linux still shells out to `cp` for
  reflink file/tree attempts.

**AC**
- Add a minimal reviewed Linux `FICLONE` wrapper or a maintained safe dependency. Probe once
  per destination and fall back without leaving a stage.
- Compare reflink, `copy_file_range`, `sendfile` and buffered copy on ext4, XFS and btrfs for
  tiny, medium, huge, sparse and cross-filesystem files. Record cache effects as well as time.
- Select by capability and measured size regime; do not use a global platform assumption.
- Remove the external `cp` dependency from per-file operation. Keep a whole-tree subprocess
  only if it wins a separate measured gate and its metadata semantics match the contract.
- Fault tests cover unsupported ioctls, partial clone/copy, source mutation and publish crash.

### Story V3.36 — Adaptive transport window, stream count and compression
- [ ] **P1/R.** Static defaults cannot be optimal across a 5.8 MB/s WAN-like route and a
  fast LAN; the current control connection also negotiates no compression in multi-stream
  mode.

**AC**
- Finish V3.18: correct capabilities on every stream, concurrent bounded setup, actionable
  `MaxStartups` handling and no single-file/small-file path divergence.
- Measure bandwidth-delay product, CPU saturation and receiver backpressure without a
  destructive link probe. Use them to select outstanding bytes, batch size, compression and
  stream count, within user-set bounds.
- Add `--bwlimit` with an aggregate token bucket across streams; observed rate stays within
  5% after ramp-up and cancellation is prompt.
- Adaptation never changes content semantics and is recorded in JSON. A deterministic override
  makes benchmarks and incident reproduction possible.
- Decision gate: automatic policy must beat the fixed one-stream default by at least 10% in
  two distinct constrained regimes and regress no retained cell by more than 5%.

### Story V3.37 — Persistent index and filesystem change journal
- [ ] **R.** This is the highest-upside latency idea: make repeated sync proportional to
  changes rather than tree size.

**Experiment / gate**
- Prototype an on-disk index plus FSEvents/inotify/USN change feed on a 1m-entry corpus.
  Target: plan a <1% change in under one second and at least 5x faster than a full scan.
- Force event loss, overflow, rename storms, journal wrap, clock changes, offline edits and
  index corruption. Every uncertainty triggers a bounded rescan; none may silently bless a
  stale index.
- Measure steady-state disk, memory, startup and update costs. Do not ship if maintaining the
  index regresses ordinary one-shot runs or requires an always-on privileged daemon.
- The index is an optimization cache, never the only copy of correctness-critical state;
  deletion or corruption must degrade to a full scan.

### Story V3.38 — Compare rsync-style delta, CDC and no-delta strategies
- [ ] **R.** V3.5 measures cross-file duplication; this story chooses the algorithm users
  would actually receive.

**Experiment**
- Compare rsync fixed-block rolling checksums, FastCDC, whole-file resend and clone/reflink on:
  one-byte edits, insertions near the front, VM images, database-like files, renamed copies,
  repeated build artifacts and incompressible media.
- Run across bandwidth/RTT regimes and include source hashing, destination reads, index size,
  memory, wire bytes, wall time and resume behavior.

**Decision gate / AC**
- Select delta only where total wall time improves at least 1.5x or wire bytes fall at least
  70% on a constrained route without a local/LAN regression above 10%.
- The cost model may choose no delta. Users can force a strategy for reproducibility.
- If CDC wins, chunk indexes are bounded, checksummed, garbage-collectable and untrusted; a
  false match can never publish corrupt bytes because the final file digest is authoritative.
- Interruption, index corruption and adversarial chunk boundaries have oracle-backed tests.

### Story V3.39 — Resource efficiency and coexistence
- [ ] **R.** A fast sync that evicts the working set or saturates the machine is not a good
  background product.

**AC**
- Measure peak RSS, open FDs, CPU, context switches, page-cache displacement and destination
  queue depth at 10k/100k/1m entries and multiple simultaneous jobs.
- Add memory/FD budgets and backpressure assertions to the benchmark harness. Bounded protocol
  frames alone do not prove a bounded whole run.
- Compare normal/low-priority I/O and CPU modes (`nice`, platform I/O priority, cache hints).
  Ship a background mode only if foreground latency improves materially and total sync time
  stays within a documented bound.
- Define behavior under system memory pressure and file-descriptor exhaustion; no busy loop or
  unbounded retry is allowed.

### Story V3.40 — Performance regression gates that can be trusted
- [ ] **P1.** The harness is sophisticated, but the release still lacks a representative
  checked-in gate baseline.

**AC**
- Nominate stable regression-tier cells for APFS and Linux, local and SSH, small-file, large,
  no-op and churn. Pin corpus digests and environment identity.
- Gate on paired ratios and phase budgets, not absolute developer-machine time. Noisy rows
  cannot pass or fail a release.
- Fail on correctness, source drift, memory/FD budget violations and a >10% regression in a
  previously selected path. Retain reports as artifacts without committing scratch trees.
- Run a lightweight PR gate and a broader scheduled/release matrix. The latter publishes the
  crossover table used by strategy policy.

---

## Part G — Additional quality-of-life work

### Story V3.41 — Config initialization, validation and effective-value display
- [ ] **P2.** Named jobs exist; make them discoverable and safe to edit.

**AC**
- `xs config init` writes a commented example only after confirmation and never overwrites an
  existing file. `xs config path` says which file would be loaded.
- `xs config check` validates every job without contacting a remote. `xs job show NAME` prints
  effective values and their origin (CLI, job, environment or default), with secrets redacted.
- Add explicit negative boolean flags where a job can otherwise enable a destructive option
  that the CLI cannot turn off, starting with `--no-delete`.
- Job-level environment interpolation, if supported, is allowlisted and visible; missing values
  fail before mutation rather than becoming empty paths.

### Story V3.42 — Inspect and clean xsync-owned state
- [ ] **P2.** Users should not need the README to find journals, cache databases, staging files
  and bootstrap remnants.

**AC**
- `xs state list` reports hash cache, resume journals, locks, staged files and persisted remote
  bootstrap binaries with sizes and last-use times.
- `xs state clean` previews exact targets and requires an explicit scope/confirmation. It only
  removes paths carrying an xsync ownership marker and never recursively targets a broad temp,
  cache, home or destination root.
- Active locks/journals are refused. Stale detection is documented and testable.
- Package uninstall instructions call the same inspection logic and distinguish binary removal
  from user-data cleanup.

### Story V3.43 — Actionable retry manifests and small-file resume
- [ ] **P2.** Partial failure code 23 says something was missed; it should also make the retry
  exact and cheap.

**AC**
- `--write-retry-list FILE` atomically records raw paths, operation, source identity and error
  class for entries not completed. A matching `--retry-from FILE` validates endpoints and source
  identity before doing work.
- The format is versioned, NUL-safe on Unix and consumable by `--files-from` without lossy text.
- Small-file batches checkpoint at bounded intervals so a reconnect does not resend an entire
  million-file run. Journal fsync policy is measured; it must not recreate the historical
  fsync-per-file regression.
- A retry never replays a successful deletion or mutation and still runs final manifest checks.

### Story V3.44 — Stable automation and terminal contract
- [ ] **P2.** Make the same command pleasant in a terminal and predictable in a scheduler.

**AC**
- Define stable exit classes for no change, successful change, partial transfer, policy refusal,
  usage/config error, timeout/interruption and integrity failure. Preserve rsync's useful code 23
  where practical without pretending every rsync code maps.
- Human progress, color and prompts require a TTY; redirected stdout/stderr never receives ANSI or
  waits for input. `--non-interactive` fails instead of prompting.
- JSON schemas are versioned and contain run/job IDs, effective policy, phase timing, byte bases,
  selected strategy and terminal outcome. Logs can be rotated externally without corruption.
- `--quiet`, `--verbose`, `--stats`, `--itemize-changes` and `--progress-json` interactions are
  table-tested. Server diagnostics appear only under explicit verbosity or in structured logs.
- Provide launchd/systemd-timer examples for V3.17 and prove concurrent scheduled runs are stopped
  by V3.13's destination lock.

---

## Recommended delivery sequence

1. **Integrity before features:** V3.20, V3.21 and V3.22. Keep `--paranoid` and sparse warnings
   prominent until their default-path replacements are complete.
2. **Bound and contain operations:** V3.12, V3.13, V3.23, V3.11 and V3.24. This produces a tool
   that stops, times out, cannot interleave destinations and cannot casually wipe a backup.
3. **Rehearse the preview:** V3.32, V3.30 and V3.31. Publish one truthful, installable release
   before expanding package channels or daemon work.
4. **Make it the daily command:** V3.14–V3.16 and V3.26–V3.27, followed by V3.41–V3.44.
5. **Close known performance losses:** V3.18, V3.33–V3.36 and V3.40. Optimize the measured phase,
   then lock the win into the regression gate.
6. **Expand archive/topology parity:** V3.25, V3.28 and V3.29.
7. **Fund structural bets only on evidence:** V3.4–V3.8 and V3.37–V3.39. A recorded "do not build"
   decision is a successful research outcome.

## First ten executable tickets

These are intentionally small enough to pull without first decomposing the whole backlog.

1. Specify the sync-v2 per-file/full-file digest fields and add corrupt-payload vectors (V3.20).
2. Add the interim sparse/free-space refusal and explicit override (V3.21/V3.24).
3. Add destination-wide lock acquisition to local, push sink and pull destination paths (V3.13).
4. Add first/second Ctrl-C behavior to the shared execution pipeline (V3.12).
5. Add connect and idle timeout plumbing to the fake-rsh integration harness (V3.23).
6. Add ENOSPC and permission fault cells that assert old-destination preservation (V3.22).
7. Add `--max-delete` plus pre-delete summary and non-interactive refusal semantics (V3.11).
8. Generate a status inventory and fix the README/DEPLOYMENT contradictions (V3.32).
9. Fix multi-stream capability negotiation and add the streams-16 refusal/backoff test (V3.18).
10. Capture the phase/first-byte baseline for tiny-file, congress-10k and one-large-file routes
    before another optimization lands (V3.33).

### Story V3.20 — Local worker count should follow the storage, not the core count

- [ ] `default_local_workers` returns `available_parallelism()` (capped at 4 on
  macOS). Measured across an 8x range of core counts, the optimum did not move
  with cores. Evidence in `BENCHMARKv2.md`.

| Host | OS | Cores | Best worker count | Cost of the current default | Past the optimum |
|---|---|---:|---:|---:|---|
| freya (7950X) | Linux | 32 | 32 | none — coincides | flat to 64 |
| orion (Pi 5) | Linux | 4 | 16-32 | **20% slower** | flat past 16 |
| MacBook (M1 Max) | macOS | 10 | 8 | **10% slower** | degrades, -6.5% at 32 |

**No single variable predicts the optimum.** orion and the MacBook wrote to the
same physical SSD and disagree (16-32 vs 8), so it is not device queue depth
alone; core count fits freya, roughly fits macOS, and badly misses orion. The
surviving claim is the negative one: `available_parallelism()` is not principled.

**macOS is qualitatively different and the existing cap was right to exist.** On
both Linux hosts, worker counts past the optimum are harmless; on macOS the curve
turns over and extra workers actively contend. `MACOS_WORKER_CAP` protects
against something real. Its *value* is what is wrong — 8 is 10% faster than 4 and
still safely below the degradation at 16.

**AC**

- `MACOS_WORKER_CAP` is raised from 4 to 8. This is the one change the data
  supports outright: measured 10% faster, and 8 is still well below where macOS
  starts degrading. Cheap, low-risk, and it does not disturb Linux.
- The Linux default is no longer a bare `available_parallelism()`. Options,
  cheapest first: a floor (`max(cores, 16)`) for local transfers; a device-aware
  value read from the destination's queue depth; or a short ramp-up probe at the
  start of a large transfer. Note that a probe is the only option that could
  account for all three hosts, since no static formula fits them.
- Any change is validated on all three platforms before it lands, and must not
  reintroduce the past-optimum degradation macOS shows at 16 and 32 workers.
- Rotational destinations are considered. 16 concurrent writers suits NVMe;
  `/sys/block/<dev>/queue/rotational` is one cheap signal.
- Whatever lands, `default_local_workers` documents its reasoning the way
  `MACOS_WORKER_CAP` does, so the next person does not have to re-derive it.

**Related:** V3.19 was withdrawn as a warm-cache artifact. This story is the real
version of that question, arrived at from cold measurements on two machines.

### Story V3.21 — Re-run the macOS 16/32-worker arms controlling for enclosure heat

- [ ] The macOS sweep ran worker counts in ascending order (1, 2, 4, 8, 16, 32)
  across ~79 minutes of continuous load. The 16 and 32 arms therefore ran last, on
  a USB enclosure that had been writing for over an hour and was hot to the touch.
  **Accumulated heat is perfectly correlated with worker count**, so the observed
  degradation past 8 workers may be thermal throttling of the enclosure rather
  than macOS worker contention. Results in `BENCHMARKv2.md`.

This matters beyond a data point: the turn-over past the optimum is the *only*
evidence that macOS behaves differently from Linux, and it is what justifies
having `MACOS_WORKER_CAP` at all. If it is thermal, macOS looks like Linux — flat
past the optimum — and the cap can be raised freely rather than merely from 4 to
8. The 4-vs-8 comparison itself is unaffected; both arms ran early.

**AC**

- **Order is not confounded with heat.** Run descending (32, 16, 8, 4) or
  randomised, not ascending.
- **Bracketed control.** Run an identical 8-worker arm first and last in the
  session. If the closing 8-worker arm is slower than the opening one, the
  session drifted — thermally or otherwise — and per-arm numbers cannot be
  compared without correcting for it. This is the single most informative
  addition and is cheap.
- **Cooling between arms.** Idle the drive to a fixed floor between reps (several
  minutes), or verify temperature has returned to baseline.
- **Measure the temperature rather than inferring it.** `brew install
  smartmontools`, then `smartctl -d sat -a /dev/diskN` for the NVMe composite
  temperature; log it immediately before and after each rep. Nothing on the box
  currently reports enclosure temperature, which is why this was invisible.
- If throttling is confirmed, the affected arms are re-measured with adequate
  cooling and `BENCHMARKv2.md` is corrected — including the claim that macOS
  degrades past its optimum, and the V3.20 table that rests on it.

**Applies more broadly.** Every long sweep in this file ran ascending and
back to back, including freya and orion. Those hosts showed *flat* curves past
the optimum rather than degradation, so throttling would only have masked further
gains rather than inventing a false decline — but the same bracketed-control
practice should be adopted for future sweeps regardless.
