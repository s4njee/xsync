# Backlog v4 — ordered plan

Written 2026-08-29, at the point where filesystem/OS testing has replaced feature
work as the main activity. This supersedes nothing: `backlogv3.md` keeps the
completed record and the long tail, and stories are referenced by their v3 ids
where they already exist.

**How to read this.** Phases are ordered and mostly sequential — each produces
information the next one uses. Within a phase, items are ordered by dependency,
not by value. Anything marked *unverified* has been built but not proven.

## Where things stand

| | |
|---|---|
| Measured platforms | macOS (M1 Max), Linux x86_64 (freya), Linux aarch64 (orion, Pi 5) |
| Measured corpora | congress (1k/10k/100k/1m), cb7, Manga |
| Not yet measured | Windows, anything |
| Known-unverified code | V3.22 metadata compression — builds and passes tests, never run against a live peer |
| Recently invalidated | The v1 wire is not frozen; several designs routed around a constraint that does not exist |

---

## Phase 1 — Finish macOS filesystem/OS testing

The machine is set up, `purge` is available, Spotlight is disabled on the target
volume, and the same USB SSD has already been measured on two Linux hosts. That
makes macOS results directly comparable, and it is cheap to finish now.

### 4.1 — Manga on macOS *(in flight)*

- [~] Large-file, incompressible workload: internal APFS -> USB APFS, 118 files /
  26.7 GiB, mean 232 MB.

Calibration put the default at 58.9 s = **464 MB/s, 51% of the drive's 913 MB/s
write ceiling** — against 4% for congress on the same machine and drive. That
contrast is the point: it bounds how much of the congress result was per-file
overhead rather than device limits.

> **First attempt discarded 2026-08-29 — the drive, not the tool.** A 5-arm sweep
> plus 3 baselines at 3 reps wrote **641 GiB to a 256 GB drive in one session**,
> 2.5x its capacity. Sustained write fell from 913 MB/s (cool, empty) to 615 MB/s
> measured afterwards, and every arm from 4 workers on sat on a ~176-180 MB/s
> floor. The within-arm drift is unambiguous: workers 1 went 79.3 -> 87.8 -> 100.0 s
> across its three reps (+26%), workers 2 went 47.6 -> 53.4 -> 99.3 s (+109%).
> Worker count and accumulated wear were confounded, and the baselines — which ran
> last, on the most degraded drive — are void with them.
>
> This is the same confound that was raised against the congress sweep and cleared
> there; congress writes 13 GB per run and its cooled re-run reproduced to within
> 1.3%. Manga writes 26.7 GB, and 24 runs of it exceeds what this drive sustains.

**Redesign required.** The corpus is too large to sweep repeatedly on a 256 GB
drive:

- Use a **bounded subset** — roughly 4-6 GiB of large files keeps the large-file
  character while cutting per-run writes 5x. APFS `cp -c` clones make staging free.
- **Randomise or reverse arm order**, and **bracket the session** with an identical
  arm first and last. The bracket alone would have caught this within minutes.
- **Verify the drive between arms**: a 4 GiB `dd` write should return to baseline;
  if it does not, stop and let the drive recover.
- Budget total bytes written per session against drive capacity, and record it.

**AC**
- Worker sweep 1-16, plus `cp -R`, `ditto` and `rsync -a` baselines. `ditto` is
  the macOS-native tool and the fairest local comparison on this platform.
- The remaining ~49% of the write ceiling is either explained or recorded as
  unexplained. With only 118 files, parallelism is bounded by file count in a way
  it never was with 1.3M files, so the worker curve should look nothing like
  congress's — if it does, that is itself the finding.

### 4.2 — cb7 on macOS

- [ ] The mixed workload: 59,311 files / 3,310 symlinks / 5.49 GiB, with 82.6% of
  files under 8 KB but 68% of bytes in the 78 files over 8 MB.

cb7 is the only corpus that exercises the small-file and large-file paths
simultaneously, and it has already been measured on Linux (55.3 s at `--streams 8`
after the batching fix). A macOS number completes that comparison.

**AC**
- Cold, `purge` before every rep, same harness as 4.1.
- Worker sweep spanning the macOS optimum found for congress (8) — cb7's mixed
  shape may move it.
- Compared against the Linux cb7 figures, with the caveat that this Mac is an
  active workstation while freya was quieted.

### 4.3 — Write up the macOS platform picture

- [ ] One section in `BENCHMARKv2.md` covering all three macOS corpora, with the
  cross-platform table.

**AC**
- States plainly where macOS is slow and where it is not: congress at 4% of
  device ceiling versus Manga at 51% is the headline, and it points at per-file
  cost rather than throughput.
- Carries the workstation caveat consistently — scaling shapes are trustworthy,
  absolute cross-machine figures are not.
- Records the 40-53% sys time observed during transfers, which is the concrete
  lead for the per-file cost and connects to the earlier syscall-attribution work
  (`Sink::destination_path` at 52% of sampled stacks).

---

## Phase 2 — The work the unfrozen wire unblocks

`backlogv3.md` carries the full note. Backward compatibility is not a
requirement, so "the wire cannot change" stops being a valid reason. Ordered so
that the cheapest verification comes first and the largest change last.

### 4.4 — Verify V3.22 metadata compression against a live peer

- [ ] **Do this before anything else in Phase 2.** The code is committed
  (`44b5f0cf`) and untested on the wire.

**AC**
- A real transfer to a real remote decodes compressed `FileBatch` and `Scan`
  frames correctly, in both push and pull directions.
- `wire_bytes` from the `finished` event is compared before and after on the same
  corpus and host. congress-100k is the natural target; the path analysis
  predicts roughly 45 MB saved per congress-1m transfer.
- `--no-compress` still produces no compression, metadata included.

### 4.5 — Carry ownership and link count in the index encoding *(unblocks V3.3)*

- [ ] The dropped-metadata preflight re-`stat`s every file solely because
  `uid`/`gid`/`nlink` do not survive the encoding.

Cost measured at ~6% of a 100k-file copy before the pass was parallelised; the
parallelisation hid it rather than removing it, and it is still real work.

**AC**
- `SourceFingerprint` carries the fields and they survive encode/decode. An
  earlier attempt was reverted precisely because they did not.
- The preflight's extra `symlink_metadata` is gone, and the syscall reduction is
  measured rather than assumed.

### 4.6 — Carry filter rules on the wire *(unblocks V3.10)*

- [ ] `--include` is currently **refused outright** for remote transfers. This is
  the most user-visible consequence of the old constraint.

**AC**
- The ordered rule set crosses the wire. `filter::encode`/`decode` and their
  round-trip tests already exist; only the wire field and the server-side
  `FilterSet` are missing.
- `--include` works remotely with the same first-match-wins semantics as local,
  including the directory-descent rule that removes rsync's `--include '*/'`
  footgun.
- Per-tree `.xsyncignore` on a remote source either works or keeps warning
  honestly.
- The fail-closed refusal is removed only once the capability is negotiated, so a
  peer that cannot represent the rules still refuses rather than approximating.

### 4.7 — Fix multi-stream capability negotiation *(V3.18 remainder)*

- [ ] The multi-stream control session negotiates `capabilities=0x0`, so it
  compresses nothing — and every small file goes over it.

**AC**
- The control session negotiates the same capabilities as the data sessions.
- `--streams 16` stops failing with `Broken pipe` (N+1 SSH connections exceed
  OpenSSH's default `MaxStartups`): cap concurrent connections, back off, or
  refuse up front with a message naming the cause.
- Connections are established concurrently rather than in a sequential
  `spawn_server_child` loop, which costs ~1.3 s at 4-8 streams.

---

## Phase 3 — Windows

The first platform with no measurements at all, and the only Tier 1 platform that
has never run the test suite green. Phase 3 is therefore mostly *making testing
possible*, and only then testing.

### 4.8 — Get Windows to a green test suite

- [x] **Done 2026-08-29. 265 tests pass, 0 fail, 2 ignored**, against the previous
  record of 13 of 24 failing. `xs.exe` builds (6.29 MB, `x86_64-pc-windows-msvc`,
  blake3 + zstd, wire v2) and the engine runs real transfers under test with 24
  workers, including resume checkpointing.

**Host:** Ryzen 9 7900X (12C/24T), 31 GB RAM, Windows 11 Pro build 26200, Visual
Studio Community 2026 (MSVC was already working — it compiled zstd's C sources
without intervention).

**Four defects fixed**, two of them regressions introduced earlier in this
session that macOS could never have caught:

1. **`xattr` was an unconditional dependency** (added for V3.3 dropped-metadata
   reporting) and does not compile on Windows. Both call sites were already
   inside `#[cfg(unix)]`; only the manifest entry was ungated. Moved to
   `[target.'cfg(unix)'.dependencies]`.
2. **`note_dropped_metadata` was defined twice** — the parallelisation refactor
   left the old pre-refactor `#[cfg(not(unix))]` arm behind. `cfg(unix)` compiles
   it out on macOS and Linux, so it was invisible until Windows compiled both.
3. **`clone_spike.rs` referenced `status` on non-Unix**, where the early `return`
   was still followed by a parsed `if status.success()` and neither `let status`
   arm exists. Restructured so each platform arm is self-contained.
4. **Unix-only research spikes blocked the workspace build.**
   `xsync-remote-spike` uses `MetadataExt`, `PermissionsExt` and
   `CommandExt::exec` pervasively. `benches/engine` is now excluded from
   `default-members` rather than ported — `cargo test --workspace` on Unix still
   runs all 19 suites, so no coverage is lost.

**Note:** SSH sessions could not run `cargo` at first — rustup's shims are
symlinks to `rustup.exe`, and Windows disables *remote-to-local* symlink
evaluation by default, so a network logon cannot traverse them. Worked around by
putting the real toolchain `bin` on `PATH` rather than changing the machine's
symlink policy.

**Still open from the original AC:**
- [ ] CI runs the Windows job and is allowed to fail the build.
- [ ] Path-semantics tests: drive letters, UNC paths, `\` vs `/`, reserved names
  (`CON`, `NUL`, `AUX`), trailing dots and spaces, and the 260-character limit.
  None of these have a Unix analogue and they are where a file copier breaks.

**AC**
- `cargo test` runs and passes on Windows, or every failure is triaged and either
  fixed or documented as an unsupported case with a reason.
- CI runs the Windows job and is allowed to fail the build — `DEPLOYMENT.md`
  notes it currently would.
- Path semantics are covered by tests: drive letters, UNC paths, `\` versus `/`,
  reserved names (`CON`, `NUL`, `AUX`), trailing dots and spaces, and the 260-char
  limit. Several of these have no analogue on Unix and are where a file-copier
  breaks.

### 4.9 — Windows filesystem behaviour

- [~] **Measured 2026-08-29 against a purpose-built NTFS fixture** (hardlink pair,
  10 MB sparse file, alternate data stream, directory symlink, junction, file
  symlink, read-only file). One fix landed; three gaps documented as
  undetectable on stable Rust.

**What works**

| Behaviour | Result |
|---|---|
| Files, directories, read-only attribute | preserved |
| Symlinks, file and directory | preserved as symlinks |
| Reflink/clone probe | declines cleanly: 0 clones, byte-copy path, no error |

**What silently loses data** — all four were completely silent before this work:

| Behaviour | Measured result | Detectable? |
|---|---|---|
| **Sparse files** | 10 MB sparse file written as 10 MB of real zeros | **Yes — now warns** |
| **Hardlinks** | pair with 2 links each arrives as 2 independent copies, 1 link each | No: `number_of_links` needs unstable `windows_by_handle` |
| **Alternate data streams** | 20-byte `:hidden` stream dropped entirely | No: needs `FindFirstStreamW` (FFI) |
| **Junctions** | silently converted to symlinks | No: needs the reparse tag, which `file_attributes` does not expose |

The three undetectable cases need either an unstable feature or `unsafe` FFI, and
this crate denies `unsafe` outside one documented exemption. They are recorded in
the `note_dropped_metadata` doc comment with their measured behaviour so the next
person does not have to rediscover them.

**Fixed:** sparse files are now reported on Windows via
`FILE_ATTRIBUTE_SPARSE_FILE`, which stable std does expose. The byte saving
cannot be quantified the way `SparseReport` does on Unix — the allocated size is
not reachable — so the warning states the consequence without inventing a number.

**Also fixed, found by this work:** on a *real* run Windows reported "ownership
was not checked: **a dry run** does not write to the destination". `Owner::probe`
returns `None` both when a dry run declines to write and when the platform has no
Unix ownership, and V3.3 conflated the two. Replaced with an explicit
`OwnershipCheck` enum: `Performed`, `SkippedForDryRun`, `Unsupported`. Windows is
`Unsupported` and stays silent, because that is not a limitation of the run.

**Still open**

- [ ] V3.1 collision detection is **untested on NTFS**. A colliding pair cannot be
  created locally — NTFS is case-insensitive, so writing `readme.md` overwrote
  `README.md` in the fixture. Testing it needs a case-sensitive *source*, i.e. a
  transfer from a Linux host. Deferred to the remote leg of 4.10.
- [ ] Hardlinks, ADS and junction conversion remain undetected. Revisit if a
  `windows-sys` dependency or an `unsafe` exemption is ever judged worthwhile;
  each is a real, silent data loss on NTFS.

**AC**
- Case-insensitivity: NTFS is case-insensitive but case-preserving, so the V3.1
  collision detection must fire correctly. It was written and tested against APFS.
- Ownership and ACLs: the V3.3 dropped-metadata reporting has no Unix `uid`/`gid`
  to compare. It must say something true on Windows rather than nothing or
  nonsense.
- Sparse files, hardlinks and junction/symlink handling are each either supported
  or reported as dropped.
- No `clonefile`/`FICLONE` equivalent is assumed; the reflink probe must decline
  cleanly.

### 4.10 — Windows benchmarks, three corpora

- [ ] congress, cb7 and Manga, matching the macOS and Linux protocol.

**AC**
- Cold-cache measurement. Windows has no `drop_caches`/`purge` equivalent, so
  this needs solving first — a corpus larger than RAM is the most reliable
  approach and is why congress-1m matters here.
- Local (internal -> USB) at minimum; remote to a Linux host if time allows,
  which also exercises the cross-platform path semantics from 4.8.
- The worker-count question gets its fourth data point. Three platforms so far
  disagree about whether core count predicts the optimum (freya yes, orion no,
  macOS roughly), and Windows has a different I/O model again.

---

## Phase 4 — Carried forward

Not ordered against each other; pulled from `backlogv3.md` because they are still
the highest-value open items once the above is done.

- **V3.20 — worker count should follow the storage, not the core count.** Four
  platforms of data will make this decidable. A runtime probe is the only option
  that fits all measured hosts.
- **V3.11 — make `--delete` survivable.** Still marked P0 in v3: permanent
  removal with no undo, no maximum and no confirmation. The only item here that
  is a data-loss risk rather than a performance or ergonomics question.
- **Same-filesystem fast path** (v3, unstarted): where both ends are the same
  filesystem, per-file work can be bypassed.
- **Verify-only mode** (v3): answer "are these two trees identical?" without
  writing.
- **One source to N destinations** (v3): read and hash once.
- **`protocol.md` framing.** It describes the wire as frozen. Either drop that or
  state that the freeze is a design goal, not a compatibility obligation —
  otherwise the next person routes around a constraint that is not there.
