# xsync v3 — research directions and daily-driver polish

Companion to [backlogv2.md](backlogv2.md) (protocol v2 and the browse surface),
[DEPLOYMENT.md](DEPLOYMENT.md) (CI, signing, packaging), and
[TUNING-TASKS.md](TUNING-TASKS.md) (performance). This file holds what none of those
cover: correctness gaps that make xsync unsafe to reach for, research worth doing,
and the ergonomics that decide whether a tool becomes the one you actually type.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

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
- [x] `xsync backup` should run a saved source, destination, and flag set.

The single largest ergonomic gap. Nobody retypes an exclude list and a destination path
daily; they write a shell alias, and then the tool's own `--dry-run` and logging never
see the real configuration.

**AC**
- A documented config format and search path, with named jobs.
- CLI flags override config; precedence is defined and tested.
- A malformed config fails at startup with a precise error, never partial application.
- `xsync --dry-run <job>` works, so a saved job can be inspected before it runs.

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
- [ ] `--delete` permanently removes files with no undo and no confirmation.

**AC**
- A `--backup`/trash mode moving deletions aside instead of unlinking.
- A summary of what will be deleted **before** deleting, and a confirmation for
  interactive runs above a threshold count or fraction.
- A refusal when the deletion set exceeds a suspicious share of the destination —
  the classic "source failed to mount, mirror wipes the backup" accident.

### Story V3.12 — Interrupt cleanly
- [ ] There is no `SIGINT` handling. Ctrl-C leaves whatever state it lands in.

**AC**
- `SIGINT`/`SIGTERM` stop dispatch, let in-flight files finish or abandon their staging
  files, print a summary, and exit with a distinct code.
- Staging files (`.xsync.tmp.*`) are removed or documented as resumable.
- A second Ctrl-C exits immediately.

### Story V3.13 — Concurrent-run safety
- [ ] Two xsync runs against one destination will interleave writes. `fslock` is already
  a dependency, used only by the resume journal.

**AC**
- A destination lock, with a clear message naming the holding process.
- `--force` for the case where the user knows the lock is stale.

### Story V3.14 — Output a human wants to read
- [ ] `--progress-json` is excellent. The human-facing output is not yet its equal.

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
- [ ] Users arrive with rsync flags in their fingers.

**AC**
- Accept `-a`/`--archive` as an explicit no-op alias with a note, since xsync's defaults
  are already archive-like — silently rejecting it is a bad first impression.
- Support or explicitly reject, with a helpful message, the flags people will reach for:
  `-v`, `--progress`, `-z`, `--partial`, `--update`, `--ignore-existing`, `--bwlimit`,
  `--max-size`, `--min-size`.
- A documented rsync-to-xsync flag table in the README.

### Story V3.16 — `xsync doctor`
- [ ] One command that explains the environment before the user hits it as a surprise.

**AC**
- Reports: reflink support at the destination (the probe already exists), remote xsync
  presence and version, protocol compatibility, SSH reachability, destination
  case/normalization behaviour (from V3.1), free space against the estimated need, and
  which metadata classes will be dropped (from V3.3).
- Exits non-zero when it finds something that would fail the real run.

### Story V3.17 — Scheduling that is not cron-by-hand
- [ ] DEPLOYMENT D6 covers the daemon. This is the smaller, nearer thing: making a
  scheduled xsync run pleasant.

**AC**
- Documented launchd/systemd-timer examples for a named job from V3.9.
- Machine-readable exit codes distinguishing "nothing to do", "transferred", "partial
  failure", and "refused", so a scheduler can act on them.
- Log rotation guidance for `--log-json`.

---

## Suggested order

**Part A first, all of it.** V3.1 is silent data loss and should be treated as a bug
rather than a backlog item. V3.2 and V3.3 are one-run-away-from-angry-user problems and
are mostly reporting work, not engine work.

Then the cheap half of Part C — V3.12 (interrupt), V3.13 (locking), V3.14 (output),
V3.15 (rsync flags) — which together are what makes the tool feel finished, and none of
which need protocol or engine changes.

Then V3.9/V3.10 (config and excludes), which are the features that turn it into
something used daily rather than demonstrated occasionally.

Part B is genuine research and should stay measurement-gated in the style TUNING.md
established: spike, decision gate, and permission to conclude "do not build this".

### Story V3.18 — `--streams` correctness and the small-file path

- [~] Three defects found while investigating the congress `--streams 8` regression
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

- [x] **Withdrawn 2026-08-29: the premise was a measurement artifact.** The
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
