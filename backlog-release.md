# Backlog — release blockers before `xs` runs on every machine

Written 2026-08-30. The question this answers is narrower than "is v1 done": it
is **what would hurt if this binary were installed on all your machines and used
daily.**

The packaging *machinery* is not the problem, though the pipeline that runs it is (R1.5). `release.yml` already builds five targets,
publishes `SHA256SUMS`, emits a build-provenance attestation, gates on a
changelog entry, and `deny.toml` is enforced in CI. `docs/verifying-downloads.md`
and `docs/release-process.md` exist and are real. **The blockers are
behavioural**, and they cluster around two themes: *what happens when a run is
interrupted*, and *what happens when versions differ across the fleet*.

---

## R1 — Hard blockers

### R1.1 — Ctrl-C leaves arbitrary state *(carried from V3.12, P0)*

- [ ] **There is no signal handling anywhere in the codebase.** No `ctrlc`, no
  `signal-hook`, no handler. `SIGINT` and `SIGTERM` terminate the process
  wherever it happens to be.

This is the top blocker because it is the most likely thing to happen. Anyone
using a sync tool interactively will interrupt one, and today that can leave:

- `.xsync.tmp.*` staging files in the destination tree, with no cleanup pass
- a resume journal describing work that is no longer in flight
- an orphaned `xs --server` on the remote (SSH usually reaps it, but that is the
  transport's doing, not ours)
- a `--delete` phase that has started but not finished

**AC**
- `SIGINT`/`SIGTERM` stop dispatch, let in-flight files finish or abandon their
  staging files, print a summary, and exit with a **distinct code** so scripts
  can tell cancellation from failure.
- **No delete phase begins after cancellation.** Deleting during teardown is the
  worst available outcome.
- Staging files are removed, or documented as resumable and actually resumed.
- A second Ctrl-C exits immediately.
- Remote children are reaped within a bounded time.

### R1.2 — `--delete` is still irreversible

- [ ] The guard landed (V3.11, partial): a run removing ≥50% of a destination of
  ≥100 entries is refused unless `--max-delete` authorises it. **That prevents
  the accident; it does not provide an undo.** An authorised or under-threshold
  deletion is permanent.

**AC**
- A `--backup`/trash mode that moves deletions aside instead of unlinking, on
  the destination filesystem so it does not turn each delete into a cross-device
  copy.
- Retention and cleanup documented; a trash that silently fills a disk is its own
  incident.
- Interactive confirmation above a threshold, with non-interactive runs never
  prompting and requiring the explicit policy they already require.

### R1.3 — `--version` reports the wrong commit *(4.24)*

- [ ] Live right now: the binary reports commit `729b5cd4f844` while `HEAD` is
  `1debb7ab`. `build.rs` caches `BUILD_COMMIT` and does not re-run when `HEAD`
  moves.

**On one machine this is cosmetic. Across a fleet it is the diagnostic.** The
first question about any misbehaving host is "which build is that?", and the
tool currently answers it wrongly. This cycle already lost an afternoon to a
version-skew mystery that `--version` should have settled in seconds.

**AC**
- `build.rs` declares `rerun-if-changed` on `.git/HEAD` and the ref it points at.
- A release build with no git metadata still produces a sane, honest string.
- A test that would fail if the commit went stale again.

### R1.4 — Mixed versions are guaranteed, and the wire is not frozen

- [ ] Rolling a binary onto every machine means **every rollout has a window
  where versions differ**, and this project changes its wire deliberately —
  `SessionConfig` gained a field twice this cycle alone.

Today a mismatch surfaces as `trailing protocol bytes: 4` or
`truncated protocol data: expected 25 bytes, received 21` — accurate, and
useless to anyone who has not read the codec.

**AC**
- A version/capability check **before** the first data frame, failing with a
  message that names both versions, both commits, and the fix.
- A documented support window: which client versions may talk to which servers.
  "Always upgrade both ends simultaneously" is a valid answer only if the tool
  says so when it is violated.
- The resume journal is versioned (`RESUME_JOURNAL_VERSION = 1`) — confirm a
  journal written by another version is rejected rather than misread, and test
  it.
- Upgrade and rollback procedure in `docs/`, including what happens to
  in-progress transfers and existing journals.

### R1.5 — CI has never been green

- [ ] Four CI runs exist in this repo's entire history. **All four failed.**
  v0.1 was tagged and released on top of a red build.

Three distinct causes, and they are not equally serious:

- **Format** — `cargo fmt --all -- --check` was red at every commit since the
  gate was added. Not a toolchain disagreement: CI's pinned 1.88 and a local
  1.97 produce byte-identical diffs. *Fixed in `80ed09b8`.*
- **`x86_64-pc-windows-msvc`** — `cargo build --workspace` fails, but **every
  error is in one file**, `benches/engine/src/bin/xsync-remote-spike.rs`, which
  takes `std::os::unix::fs::MetadataExt`, `PermissionsExt`, and
  `process::CommandExt` unconditionally. The shipped crates are fine; a bench
  spike binary is breaking the workspace build on a target we release for.
  Gate the bin on `#[cfg(unix)]` with a stub `main` elsewhere.
  (`clone_spike.rs:14` also warns on an unused `set_symlink_file_times` on
  Windows — a warning, not part of the failure.)
- **The two runs on 2026-08-25** timed out at **24h0m** on a runner label that
  no longer exists. `timeout-minutes: 45` was added afterwards, so this should
  now fail in 45 minutes rather than a day — but it has not been re-observed,
  so treat the runner matrix as unverified.

**A red CI is worse than no CI**, because it stops being read. Nothing else in
this document can be trusted to stay fixed until the pipeline is green and
kept green.

**AC**
- All jobs green on `main`.
- A release tag cannot be published from a red build.

---

## R2 — Fix before wide deployment

### R2.1 — Memory at scale is unmeasured *(4.23)*

- [ ] The planner holds both trees in memory, plus batch data and the pipeline.
  congress-1m has run without incident, but **peak RSS has never been recorded
  on either end.** The fleet includes a 3 GB Raspberry Pi, and the receiver now
  spawns an apply pool.

"It did not OOM once" is the entire current knowledge. Measure before deploying
to the smallest machine, not after.

### R2.2 — Windows drops metadata it cannot see *(4.11)*

- [ ] Hardlinks and alternate data streams are undetected on NTFS. They are now
  *reported* as unchecked rather than silently lost, which closed the dangerous
  half. But a hardlinked pair still arrives as two independent copies.

Acceptable for a release **if documented in the README's platform notes**, which
it currently is not. A user should not discover this from a diff.

### R2.3 — Bootstrap uploads the wrong architecture

- [ ] `--bootstrap` uploads `std::env::current_exe()` — the *local* binary. From
  a macOS laptop to a Linux server, that is an executable that cannot run.

**AC**: select by the probed remote `uname`, or refuse with a clear message
rather than uploading something that will fail on exec.

### R2.4 — `wire_bytes` is not the bytes on the wire *(4.44)*

- [ ] It counts data frames only; every `encode_meta_frame` site writes to the
  transport without adding to it. The summary line and JSON both understate real
  traffic — 0.9% on congress, more on metadata-heavy corpora.

A user-visible number that is wrong. Split it or fix it.

### R2.5 — Pull always asks the remote to checksum everything *(4.46)*

- [ ] `run_client_pull` hardcodes `checksum: true` while push passes
  `options.checksum`. Every pull makes the remote BLAKE3 every file regardless of
  what was asked.

Still not established as a bug — it may be load-bearing for pull-side
classification. **Determine which before shipping**, because if it is not
load-bearing it is a large, silent, per-file cost on half of all operations.

### R2.6 — The docs disagree with the measurements

- [ ] `docs/OS.md` still carries figures that `BENCHMARKSv3.md` superseded: 99.8 s
  and 1,099 files/s for Windows, and "the operating system is worth ~6×". The
  current measured figures are far better in absolute terms and **3.32×** for the
  ratio, and part of that former gap was our own serialised receiver.

Shipping documentation that contradicts the project's own benchmark file is a
credibility problem, not a formatting one.

---

## R3 — Operational, once deployed

### R3.1 — CI and release do not build the same set

- [ ] CI covers **seven** targets including both musl variants; `release.yml`
  builds **five** and omits musl entirely.

Either musl is supported — in which case it should be released — or it is not,
in which case CI is spending time on something users cannot obtain. Decide, and
make `docs/TARGET-MATRIX.md` the single answer.

### R3.2 — No fleet-wide upgrade story

- [ ] Installing on every machine implies a way to keep them aligned. There is a
  release process and an install script; there is no answer to "which of my
  hosts are on the old build".

**AC**: at minimum, `xs --version` output that is machine-readable and correct
(depends on R1.3), and a documented one-liner to report it across hosts.

### R3.3 — Failure diagnosis in the field

- [ ] `--log-json` exists and appends, which is right. What is missing is
  guidance: what to capture when a transfer misbehaves, and what to send.

`docs/failure-log-v1.md` documents the format. A short "reporting a problem"
section pointing at it would make the difference between a useful report and a
screenshot.

---

## What is explicitly *not* a blocker

Recording these so they are not re-litigated at release time:

- **Performance.** 3.11× end-to-end this cycle, at the wire on a Linux pair, and
  ahead of `rsync` in every measured configuration except a Pi sender. The two
  remaining known items are ~10% each.
- **The transport.** SSH measures indistinguishable from raw TCP; there is
  nothing to win there and no reason to hold a release for it.
- **io_uring, multiplexed streams, hash parallelism.** All measured and declined
  on evidence.
- **`--streams` on Windows.** Harmful, measured, and the default is 1.

---

## Suggested order

1. **R1.5 green CI** — everything below is unenforceable without it, and the
   remaining piece is one `#[cfg(unix)]` on a bench binary.
2. **R1.1 signal handling** — most likely to bite, and it can leave a mess.
3. **R1.3 `--version`** — small, and everything else's diagnosis depends on it.
4. **R1.4 version skew** — required before the *second* machine gets a binary.
5. **R2.1 memory at scale** — a measurement, not a change; do it before the Pi.
6. **R1.2 `--delete` backup** — the largest piece of work here, and the guard
   already prevents the catastrophic case, so it need not block the first
   install on machines you control.
7. Everything else.
