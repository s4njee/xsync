# Backlog v4 — ordered plan

Written 2026-08-29, at the point where filesystem/OS testing has replaced feature
work as the main activity. This supersedes nothing: `backlogv3.md` keeps the
completed record and the long tail, and stories are referenced by their v3 ids
where they already exist.

**How to read this.** Phases are ordered and mostly sequential — each produces
information the next one uses. Within a phase, items are ordered by dependency,
not by value. Anything marked *unverified* has been built but not proven.

## Where things stand

*Table refreshed 2026-08-30.*

| | |
|---|---|
| Measured platforms | macOS (M1 Max), Linux x86_64 (freya 7950X; mars 7900X), Linux aarch64 (orion, Pi 5), Windows 11 (7900X — the same physical box as mars, dual-boot) |
| Measured corpora | congress (1k/10k/100k/1m), cb7, Manga |
| Not yet measured | Anything above ~1 GbE; any x86 that is not Zen 4; XFS and btrfs; WiFi; BSD. See 4.18. WSL2 is enabled but has no distro — 4.47 |
| Known-unverified code | None. Phase 2 is complete: 4.4 verified V3.22 on the wire, 4.5, 4.6 and 4.7 are landed and measured |
| Recently invalidated | The v1 wire is not frozen. `wire_bytes` is not the bytes on the wire (4.44). The preflight's "frozen wire" comment blamed the wrong encoding (4.5). The ~1.3 s sequential-spawn cost did not reproduce (4.7) |
| Measured but not yet trustworthy | 4.15's 1.86× on freya was taken under ~950% competing CPU. The ratio reproduced five times; the absolute figures did not match an idle run of equivalent code. See 4.49 |

**The result the next few phases exist to act on.** Small-file network sync is
bound by neither endpoint: both sit near 50% CPU, and a Pi 5 receives within 7%
of a 7950X. The sender's batch builder is serial and phase-separated — it issues
up to 8,192 blocking reads with the network idle, then hashes, compresses, and
frames with the disk idle. 4.15 fixes the sender, 4.26 the receiver, and 4.25
notes that `--streams` currently buys *zero* parallelism on small-file corpora
because everything under `MAX_DATA_SEGMENT` rides the control session.

**Numbering note.** Phase 10 owns 4.29–4.43 and cross-references its own ids, so
the two stories filed from the 4.4 and 4.5 work took 4.44 and 4.45 rather than
renumbering it.

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

## Phase 2 — The work the unfrozen wire unblocks *(complete 2026-08-30)*

`backlogv3.md` carries the full note. Backward compatibility is not a
requirement, so "the wire cannot change" stops being a valid reason. Ordered so
that the cheapest verification comes first and the largest change last.

### 4.4 — Verify V3.22 metadata compression against a live peer *(done 2026-08-30)*

- [x] Verified against the Windows host (`sanjee@192.168.1.120`, MSVC build
  carrying `encode_meta_frame`) on congress-100k — 109,615 files, 583,940,018
  logical bytes.

**AC1 — compressed `FileBatch`/`Scan` decode in both directions: PASS**

- Push Mac → Windows: 109,615 transferred, 0 failed. The Windows peer decoded
  compressed metadata frames.
- Pull Windows → Mac: 109,615 transferred, 0 failed, 0 warnings. This is the
  stronger half — the **Windows** binary encoded the compressed frames and the
  Mac decoded them.
- Independent round-trip content digest (sorted per-file SHA-256 over the whole
  tree) identical to the original source: `b6114e969f2f7997…`.

**AC2 — `wire_bytes` before/after: THE AC WAS NOT SATISFIABLE. Measured directly instead.**

`wire_bytes` **excludes metadata frames entirely.** Only `write_data_frame*`
and `decoder.last_wire_bytes()` feed `report.wire_bytes`; all four
`encode_meta_frame` call sites do a bare `writer.write_all(&bytes)` and never
add to it. An A/B of two purpose-built binaries — one with `encode_meta_frame`
forced to `CompressionMode::None` — returned **byte-identical** `wire_bytes` of
139,736,860 on both arms, which measures the metric's blindness, not the
feature. Filed as 4.44.

Measured by instrumenting `encode_meta_frame` to encode both ways and report
the sizes (throwaway build, not committed):

| | congress-100k push |
|---|---:|
| Metadata frames | 19 |
| Uncompressed | 9,957,243 B |
| Compressed (sent) | 1,255,326 B |
| **Ratio / saved** | **7.93× / 8.70 MB** |
| Data frames (`wire_bytes`) | 139,736,860 B |
| Share of total wire traffic saved | 5.8% |

**The commit's own predictions were both wrong, in opposite directions.** It
claimed 15.5× and ~45 MB per congress-1m; actual ratio is **7.93×** (about
half), while the volume extrapolates to **~87 MB** per 1m files (about double).
The 15.5× figure came from a path-only sample; real frames carry sizes, modes,
and mtimes that compress far less well than shared path prefixes.

**No measurable time effect**: 97.8 s (on) vs 98.2 s (off). Expected — 8.7 MB
against 140 MB on a link that 4.15 established is sender-bound, not
bandwidth-bound. This feature is a bandwidth win, not a latency one, and should
be described that way.

**AC3 — `--no-compress` suppresses metadata compression too: PASS**

- Metadata sent = 9,957,243 B, exactly the uncompressed size.
- Data `wire_bytes` = 593,147,678 vs 583,940,018 logical — framing overhead
  only, no compression.

### 4.5 — Carry ownership and link count in the index encoding *(done 2026-08-30)*

- [x] `SourceFingerprint` now carries an optional `UnixMetadata { uid, gid,
  nlink }`, populated from the scan's own `stat`, and the preflight reads it
  instead of re-`stat`ing every planned file.

**Why the earlier attempt failed.** Source entries pass through
`PlanningSpool`, which spills to disk past an 8 MB budget. Fields added to the
fingerprint but not to the record encoding therefore survive for small trees
and vanish for large ones — a silent, size-dependent bug. The fix extends
`write_entry`/`decode_entry`: `RECORD_FIXED_BYTES` 46 → 47 for a second
presence flag, plus a 16-byte block written only when present. Two tests pin
it, one of them over every combination of the two independent optional blocks,
because a mismatch between the written length and the expected length is
rejected as store corruption rather than ignored.

**Scope.** The wire is not involved. The push source is always local, and pull
does not run the preflight, so only the on-disk planning record needed the
fields. Entries rebuilt from a peer's index carry `None`, which is correct —
those uids describe another host. Any entry arriving without the block still
falls back to a `stat` rather than silently reporting nothing.

**AC1 — fields survive encode/decode: PASS.** Unit round trip, plus end-to-end
on a 40,000-file tree with deliberately long paths (~14.5 MB of records against
the 8 MB budget, so the spool genuinely spilled). Hardlink reporting is
byte-identical to the pre-change binary: `100 hardlinked file(s) become
independent copies, adding 3.1 KiB`.

**AC2 — syscall reduction measured, not assumed: PASS.** Two independent
methods agree exactly. In-process counters on congress-100k:
`entries=109615 fallback_stats=0 sparse_probe_stats=6`. `strace -f -c` on freya,
same corpus, before and after builds differing only in these four files:

| stat-family syscalls | before | after |
|---|---:|---:|
| `statx` | 516,060 | 406,445 |
| `newfstatat` | 161,339 | 161,339 |
| **total** | **677,399** | **567,784** |

The delta is **109,615** — exactly the file count. 16.2% fewer stat-family
syscalls overall. Both builds printed the identical six sparse warnings,
matching the six sparse probes the in-process counter recorded.

**Still spent per run**: one `symlink_metadata` for files at or above
`SPARSE_PROBE_MIN_BYTES` (6 files here), and `xattr::list` per entry, which is
not in the fingerprint. The strace timings are not a wall-clock claim —
tracing inflates syscall cost by roughly an order of magnitude.

**Follow-up not taken**: the Windows arm of `note_dropped_metadata` still stats
every file for `FILE_ATTRIBUTE_SPARSE_FILE`. The same treatment would need
`file_attributes` carried in the record. Filed as 4.45.

### 4.6 — Carry filter rules on the wire *(done 2026-08-30)*

- [x] `--include` now works against a remote. The ordered rule set crosses the
  wire in a new `filter_rules` field on `SessionConfig`, gated by
  `CAP_FILTER_RULES` (`1 << 4`).

**AC1 — the ordered rule set crosses the wire: PASS.** `filter::encode`/`decode`
already existed; this added the wire field, the capability, and the server-side
`FilterSet`. The two representations are mutually exclusive and the decoder
*rejects* a message carrying both, so a receiver never has to guess which one
describes the transfer.

**AC2 — `--include` works remotely with local semantics: PASS.** Verified
against freya with matching binaries on both ends, on a tree of 7 files:

| | result |
|---|---|
| local `--include 'keep/**' --exclude '*'` | `keep/a.txt`, `keep/b.log`, `keep/nested/deep.txt` |
| push to freya, same filter | identical |
| pull from freya, same filter | identical |
| push `--exclude '*.log'` | both `.log` files dropped, rest transferred |

The directory-descent rule holds across the wire: `keep/nested/deep.txt`
arrives without anyone writing rsync's `--include '*/'`.

**AC3 — `.xsyncignore` on a remote source keeps warning honestly: PASS.** The
note still fires; only the include *refusal* moved.

**AC4 — fail-closed until the capability is negotiated: PASS.** The refusal
moved out of argument parsing, where the peer is unknown, to the two places
that actually know: `server::filter_for_peer` refuses a peer that does not
advertise `CAP_FILTER_RULES`, and `rsync::validate_options` refuses the rsync
fallback, which applies `exclude_patterns` only and would otherwise silently
transfer a wider set. Refusing at parse time as well would have rejected
include rules against peers that honour them perfectly.

**Wire compatibility was deliberately not preserved.** An intermediate version
made `filter_rules` an optional trailing block so pre-capability binaries kept
working. That was removed: this is a greenfield project with no deployed users,
and the compatibility machinery bought nothing while complicating the codec.
Peers must be rebuilt, which is what `--version` skew reporting and the D5.2
bootstrap exist for.

**Incidental fix.** The four client-side filter sites rebuilt the entire
pattern vector *for every entry scanned* — an allocation per file. They now
share one hoisted `FilterSet`.

### 4.7 — Fix multi-stream capability negotiation *(done 2026-08-30)*

- [x] The control session advertised `capabilities: 0` and hardcoded
  `CompressionMode::None`. It carries the plan and **every small file**, so the
  whole small-file path was uncompressed whenever `--streams > 1`.

**AC1 — the control session negotiates like the data sessions: PASS.** It now
advertises `CAP_ZSTD | CAP_VERSION_NEGOTIATION | CAP_FILTER_RULES` and uses the
negotiated compression for its batched small-file sender, which was passed a
hardcoded `false`. Measured on a 5-file congress subtree over `--streams 2`:
**557,868 logical → 78,999 wire bytes, 7.06×**, where the previous code could
not compress at all.

**A third bug, found while testing, worse than the one filed.** The control
session also sent `filter_rules: Vec::new()` plus the raw exclude list, so the
server applied `--exclude '*'` to its *own destination scan*. The destination
index came back empty, every file looked new, and **multi-stream re-transferred
everything on every run**:

| `--streams 2 --include 'keep/**' --exclude '*'`, run twice | before | after |
|---|---|---|
| second run | 3 transferred, 0 skipped | 0 transferred, **3 skipped** |
| single-stream control | 0 transferred, 3 skipped | unchanged |

The transferred *set* was always right — source filtering is client-side — so
this was invisible except as work redone every time.

**AC2 — `--streams 16` stops failing: PASS.** OpenSSH's default `MaxStartups`
is `10:30:100`, and 16 streams plus the control session opens 17 connections
that all authenticate at once. Measured before: **2 of 3 runs failed** with
`server stream disconnected`. A `ConnectionGate` now bounds concurrent
establishment to 8. After: **10 of 10 runs pass** (5 with a trivial file, 5
with a 96 MB striped file). The permit is released as soon as the peer's
handshake proves authentication finished, not when the transfer ends — holding
it longer would have capped transfer concurrency rather than establishment.

**AC3 — concurrent establishment: DONE, but the stated cost was not
reproducible.** Each thread now opens its own connection instead of the main
thread spawning them in a sequential loop. The ~1.3 s figure did not reproduce
on this path: `spawn_server_child` only calls `Command::spawn`, which does not
block on the SSH handshake, and the handshakes already ran inside the data
threads. Measured setup, 96 MB file:

| streams | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| before | 1.62 | 1.62 | 1.61 | 1.63–1.91 | 2 of 3 failed |
| after | 1.71 | 1.68 | 1.56 | 1.56 | 1.66 |

Flat in both, so there was no sequential-spawn penalty to remove here. If the
1.3 s was real it was measured elsewhere — Windows process spawn is the
plausible candidate, and 4.10's Windows numbers would settle it.

**Not addressed**: streams still do not help this workload at all — 96 MB moves
in ~1.6 s at every stream count, so the link is the limit. That is 4.14's
question, not this one.

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

**None of these fail silently any more.** Preservation is not required for
feature-completeness; silence is the actual defect, because it reads as "you had
none" when it means "I cannot see them".

- **Junctions are now counted.** Reparse points are visible through
  `FILE_ATTRIBUTE_REPARSE_POINT` even though their *kind* is not, so the run
  reports how many will be recreated as symlinks.
- **Hardlinks and ADS are declared unchecked.** `Preflight::unchecked` carries the
  categories this platform cannot inspect, reported once per run in the summary
  and as `unchecked_metadata` in the `finished` event. The message says xsync
  cannot detect them *here* — not that the source had none.

Their measured behaviour is also recorded in the `note_dropped_metadata` doc
comment, so the next person does not have to rediscover it.

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
- [ ] Hardlinks and ADS remain *undetected* (though no longer unreported).
  Detecting them needs the unstable `windows_by_handle` feature or
  `FindFirstStreamW` via FFI. Revisit only if a `windows-sys` dependency or a
  second `unsafe` exemption is judged worthwhile — the reporting now makes the
  gap visible, which was the actual problem.

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

- [~] congress, cb7 and Manga, matching the macOS and Linux protocol.
  congress-100k in progress.

> **The Windows box and `mars` are the same physical machine, dual-booting.** The
> 1.9 TB partition on the SPCC NVMe (disk 1) is an Arch install; Windows lives on
> the Kingston NVMe. **Never format or mount disk 1** — it is a live system with
> the owner's data. The RAW Inland SATA SSD (disk 0) is the one earmarked for a
> second volume, and the owner is formatting it themselves.
>
> This makes the Windows-versus-Linux comparison far stronger than it looked:
> same Ryzen 9 7900X, same RAM, same board, same network. The earlier
> `Mac -> mars` figures and today's `Mac -> Windows` figures differ by operating
> system rather than by hardware.
>
> | | Same corpus, congress-100k, from the same Mac |
> |---|---|
> | `Mac -> mars` (Arch Linux) | 17.3 s, ~6,336 files/s |
> | `Mac -> Windows` | 106.9 s, 1,025 files/s |
>
> **6.2x, on identical silicon.** One caveat to carry into the writeup: the two
> operating systems live on *different NVMe drives* (Kingston for Windows, SPCC
> for Arch), so the storage is not controlled even though the machine is. Whether
> that accounts for any of the gap is untested; Defender being the top CPU
> consumer during the Windows run (`MsMpEng`, 4.5x the next process) points
> elsewhere.

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

### 4.12 — Measure the Windows Defender tax on file-copy throughput

- [ ] **Medium priority.** Real-time protection scans every written file. On a
  109,615-file corpus that is a per-file cost paid on top of everything xsync
  does, and it is the leading hypothesis for why Mac -> Windows staged at
  **1,025 files/s** against 6,351 files/s for the same corpus Mac -> Linux.

**This is not a recommendation to disable it.** Nobody realistically turns
Defender off, so the default configuration is the honest one to benchmark and
report. The value of measuring the delta is explanatory: it separates "xsync is
slow on Windows" from "Windows file creation is expensive under real-time
scanning", and gives the writeup a number to point at instead of a hypothesis.

**AC**
- Same corpus, same host, two arms: stock Defender, and with a temporary
  `Set-MpPreference -ExclusionPath` covering only the benchmark source and
  destination. The exclusion is a **security setting change and needs the
  operator's explicit consent** — it is not something the benchmark harness
  should do on its own.
- The exclusion is removed afterwards, and the writeup states it was temporary.
- Results are reported as "Windows as configured" (the headline) and "Windows
  with the benchmark paths excluded" (the explanatory figure). The first is the
  number users will experience.
- If the gap is large, it belongs in the README's platform notes, since it
  affects how xsync's Windows numbers should be read against Linux and macOS.

**Related, found while setting this up:** the machine's existing Defender
exclusions point at `C:\Users\sanje\...` — the pre-local-account profile path.
They no longer match `C:\Users\sanjee\...`, so cargo builds and everything else
in the current profile are being scanned. Worth fixing on the box independently
of this story.

## Phase 5 — Very low priority

Recorded so they are not lost, explicitly *not* scheduled.

### 4.13 — `--streams` was broken against Windows servers *(fixed 2026-08-29)*

- [x] Multi-stream transfers to a Windows host failed outright with
  `transport error on stream 0: server stream disconnected`.

**Cause.** Only the single-stream path discovers the remote shell family. It
tries the POSIX form, retries as `RemoteShell::Windows` on
`RemoteShellMismatch`, and caches the answer with `remember_remote_shell`;
later `spawn_server_child` calls read that cache. A fresh process invoked with
`--streams N` never runs that discovery — it assumed POSIX, cmd.exe could not
parse the single-quoted command, every child exited, and the error named the
symptom rather than the cause.

**Fix.** `ensure_remote_shell_known` probes once before the multi-stream control
session spawns, and caches the result so the control session and all data
threads agree.

**A mistake worth recording.** The first version of that probe spawned a real
`xs --server` against the destination and killed it, which left the destination's
lock and journal state behind and broke `--streams` to Linux, which had been
working. It now runs a harmless marker command through the same quoting path and
never touches the destination. Verified afterwards across all four combinations:
single and multi-stream, to Linux and to Windows.

**Also found:** freya's `~/.local/bin/xs` was three days stale and spoke an older
wire version, which produced `version mismatch: local v1 / remote v2` and briefly
looked like a third bug. Remote binaries need updating alongside the client when
the wire changes — worth a check in the benchmark harness rather than a surprise
mid-run.

### 4.14 — Choose the stream count from the corpus, not from the user

- [ ] `--streams` defaults to 1 and is otherwise whatever the user typed. The
  measurements say the right value is a property of the *corpus*, and xsync
  already knows it by the time it could act.

**Why this is structural, not just empirical.** The multi-stream path partitions
on `MAX_DATA_SEGMENT` (8 MB): files at or below it are written by the **control
session**, and only larger files are striped across data threads. Streams can
therefore only ever accelerate large-file bytes. Every measurement agrees:

| Corpus | Bytes in files >8 MB | Measured effect of streams |
|---|---:|---|
| congress-100k | 0% | harmful — Linux neutral after the batching fix, Windows 1.44-1.53x slower |
| cb7 | 68% | untested as a streams question; its 2.70x was the batching fix |
| Manga | 99% | helps, ~1.2x, peaking at 4 |

**Gate on bytes, not file count.** cb7 is 82.6% small files by count but 68% large
by bytes. A count-based rule would disable streams on exactly the corpus most
likely to benefit.

**AC**

- The default stream count is derived after planning, when the size distribution
  is already known, from the share of transferred **bytes** in files above
  `MAX_DATA_SEGMENT`. Below a threshold, 1 stream. Above it, a small number.
- **An explicit `--streams N` always wins.** The heuristic sets a default, never
  overrides an instruction.
- **The fixed cost is charged honestly.** Each stream is an extra SSH connection:
  measured at ~1.3 s for 4-8 streams on Linux and higher on Windows, established
  in a sequential `spawn_server_child` loop. Streams should only be chosen when
  the expected gain exceeds that, which for a ~1.2x gain means a transfer of at
  least tens of seconds. A small large-file corpus should still use 1.
- **Cap by platform.** Windows made streams *harmful* even on small files, and its
  per-connection cost is higher. The cap there should be lower than on Linux, or
  1 until measured otherwise — the same shape of platform-specific limit as
  `MACOS_WORKER_CAP`, and for the same evidence-led reason.
- The chosen value is visible: reported in the run summary and the `finished`
  event, so a surprising choice can be diagnosed rather than guessed at.

**Evidence gap to close first.** The large-file benefit is measured on Linux
only. A Manga-over-network run to Windows is queued; if streams turn out to be
harmful there even on large files, the platform cap becomes the dominant term and
the byte-share rule applies only to Unix targets.

### 4.11 — Detect hardlinks and alternate data streams on Windows

- [ ] **Very low priority.** These are undetected on NTFS but no longer
  unreported: `Preflight::unchecked` names them every run, so a user is told
  xsync cannot see them rather than being told nothing. The reporting gap — the
  part that actually risked data loss going unnoticed — is closed.

Detection would require one of:

| Category | What it needs | Cost |
|---|---|---|
| Hardlinks | `std::os::windows::fs::MetadataExt::number_of_links`, behind the unstable `windows_by_handle` feature | pins the project to nightly, or a `windows-sys` dependency plus `unsafe` |
| Alternate data streams | `FindFirstStreamW` / `FindNextStreamW` | a `windows-sys` dependency plus `unsafe` |
| Junction vs symlink | reading the reparse tag (`FSCTL_GET_REPARSE_POINT`) | same |

All three mean either nightly Rust or a **second `unsafe` exemption**, against a
crate that currently has exactly one and documents it prominently. Measured
consequences if a user does hit them: a hardlinked pair arrives as two
independent copies, an alternate data stream is dropped, and a junction becomes a
symlink.

**Only take this on if** a real workload is known to depend on it. The
cost-benefit is poor otherwise: an `unsafe` exemption is permanent, and the
current reporting already prevents the silent-loss failure mode.

## Phase 6 — The small-file sender

### 4.15 — Overlap read and send in the small-file batch loop *(done 2026-08-30)*

- [x] The batch builder was serial and phase-separated: it issued up to 8,192
  blocking reads with the network idle, then hashed, compressed and framed with
  the disk idle. A loader thread now runs one batch ahead over a bounded
  channel, reading each batch across `options.local_workers` threads.

**A/B, congress-100k, alternating arms, same binaries throughout.**

| target | before | after | gain |
|---|---|---|---|
| freya (7950X, Linux) | 24.58, 24.75, 24.88, 25.22, 30.06 → **24.88 s** | 11.88, 11.89, 13.36, 13.46, 15.11 → **13.36 s** | **1.86×** |
| Windows 11 (7900X, NTFS) | 97.45, 97.04 → **97.2 s** | 84.53, 83.92 → **84.2 s** | **1.15×** |

**The two numbers disagree for a reason.** Windows is receiver-bound — its
metadata path caps it near 1,300 files/s — so unblocking the sender can only
recover the sender's share. freya has headroom, so it recovers much more. That
is the shape 4.26 predicts: the sender fix and the receiver fix multiply, and
Windows will not move much until the receiver-side work lands.

**Caveat on the freya numbers, stated rather than hidden.** freya was running
`xc` at ~950% CPU throughout, so both arms were measured under heavy load. The
ratio is reproducible (five paired runs, the after arm landing 11.88/11.89 back
to back) but the absolute figures are inflated: equivalent code measured 14.4 s
on an idle freya earlier in the cycle. **Re-measure on an idle host before
quoting 1.86× as the platform number.**

**AC checks**

- Ordering deterministic: read results stay index-aligned with the plan, so
  failures and successes surface in the order a serial read would produce.
- Correctness: full-tree content digest after a read-ahead push matches the
  source exactly (`b6114e969f2f7997…`), the same digest 4.4 recorded.
- Large-file and `--streams` unregressed: 96 MB striped file, 1.59 s / 1.68 s
  before against 1.59 s / 1.61 s after at 1 and 4 streams.

**Also fixed here**: `sparse.rs` imported `FileIdentity` and `UnixMetadata`
unconditionally while using them only in the Unix arm, so 4.5 had left a
`unused_imports` warning that only appears on Windows — where CI builds with
`-D warnings`. Found by building the A/B arms on the Windows host.

## Phase 7 — Research program

Three research stories. None block Phase 6; each exists because current
conclusions rest on a narrower evidence base than the writeups imply.

### 4.16 — Corpus diversity: do the conclusions survive other data shapes?

- [ ] Every tuning decision so far rests on three corpora: congress (many small
  compressible JSON/text), cb7 (mixed + 3,310 symlinks), Manga (118 large
  files). That is three points in a much larger space, and congress compresses
  4.2× — wire-throughput numbers are flattered by it.

Gaps worth a corpus each:

- **Incompressible small files** (thumbnails, compressed images) — strips the
  zstd advantage and stresses the frame path per byte.
- **node_modules-shape** — hundreds of thousands of tiny files in deep trees;
  stresses metadata pipelining and directory creation more than data.
- **Git working trees** — mixed sizes, many dirs, hardlink-adjacent patterns;
  the shape most developer users will actually sync.
- **Pathological names** — long paths, unicode, spaces; correctness sweep more
  than performance, but cheap to add to the same harness.

**AC**: corpora are reproducibly generated (script in `benches/`) or pinned
public downloads; the streams byte-share rule (4.14), the pipeline window
(2048), and the worker-count findings are re-tested against them; any
conclusion that flips is recorded in `docs/STREAMS.md` / `docs/OS.md`.

### 4.17 — SSH transport: how much is the tunnel costing?

- [ ] Measured but unexplained: **5.3 ms in-session round trip** through an SSH
  pipe on a LAN where raw RTT should be sub-millisecond, **195 ms** per session
  setup, and **~1.3 s** to spawn 4–8 stream connections sequentially.

Angles, cheapest first:

- **Quantify the tax**: same host pair, same corpus streamed over raw `nc` vs
  through `ssh cat`. One afternoon; bounds every other item.
- **Why 5.3 ms**: sshd channel windowing, Nagle on the sshd side, or packet
  scheduling. If it's windowing, larger `MAX_PIPELINED_FRAMES` compensates
  (already done) but a root fix may lower it for everyone.
- **Per-stream setup**: `ControlMaster`-style multiplexing or parallel spawn
  instead of the sequential `spawn_server_child` loop (ties into 4.7/V3.18).
- **Cipher choice**: aes-gcm vs chacha20-poly1305 on hosts with and without
  hardware AES (Pi 5 has none). Only matters if the raw-vs-ssh delta is large.
- **Out of scope for v1**: QUIC or a native TLS daemon transport — that is the
  v2 daemon conversation, and this story's numbers are its justification.

**AC**: a table in `docs/` giving files/s and MB/s for raw-TCP vs SSH on the
same pair, the explanation for the 5.3 ms RTT, and a go/no-go on multiplexed
stream setup with measured setup times.

### 4.18 — Platform and hardware matrix: where is the map blank?

- [ ] Current coverage is deep but narrow: macOS on Apple Silicon, Linux on two
  Zen 4 desktops and a Pi 5, Windows on one Zen 4 desktop, all on a ~1 GbE LAN.
  The OS conclusions in `docs/OS.md` (OS worth ~6×, CPU ~0%) are strong on this
  hardware and untested off it.

Blank areas, in priority order:

1. **Faster links** — at 2.5/10 GbE does the sender bottleneck (4.15) simply
   dominate everything, or do new ceilings appear? Cheapest decisive test of
   whether tuning generalizes.
2. **Slower/older x86** — everything x86 measured is Zen 4. A ~2015 laptop
   would say whether "CPU ~0%" holds when the CPU is actually slow.
3. **Filesystems** — ext4 and APFS are characterized; ZFS was abandoned for
   variance; XFS and btrfs (CoW!) are unmeasured on identical hardware.
4. **WiFi** — high-RTT, lossy links exercise the pipeline window in the
   opposite direction from the LAN; the 2048 knee was measured at 5.3 ms RTT
   and may be wrong at 30 ms.
5. **BSD** — correctness first (does the suite pass on FreeBSD?), performance
   second.

WSL closes part of this cheaply: 4.48 adds two more OS paths on hardware
already in the fleet, without buying anything.

**AC**: not exhaustive coverage — a prioritized matrix in `docs/OS.md` marking
each cell measured / inferred / unknown, plus measurements for (1) and (2),
which are the two most likely to change engineering decisions.

## Phase 8 — Gaps found in review (2026-08-29)

Each of these was checked against the code or reproduced before being written
down. Suspected gaps that turned out to be covered — CI (matrix exists in
`ci.yml`), protocol fuzzing (`fuzz/fuzz_targets/protocol.rs`), read failures
silently dropped (they emit `Failed` and count) — are deliberately absent.

### 4.19 — A `chmod` never syncs: mode is not part of change detection

- [ ] **Correctness gap, reproduced.** `metadata_matches` (`planner.rs:697`)
  compares kind, size, and mtime — never mode. Repro: sync a file, `chmod 600`
  the source, preserve its mtime, sync again → destination stays 644 and the
  run reports nothing to do. `rsync -a` repairs this; xsync's archive-like
  defaults claim permission preservation and silently don't deliver it for
  drift after the initial copy.

**AC**

- Mode-only drift is detected on Unix-to-Unix syncs and repaired with a
  **metadata-only operation**, not a content retransfer.
- Windows endpoints are exempt where modes are synthesized (`permission_mode`
  invents 0o755/0o644): comparing a synthesized mode to a real one would make
  every file perpetually "changed". The synthesized-mode case must classify
  as unchanged.
- `--checksum` mode gets the same repair — content hash match must not skip
  the mode comparison.
- Ownership drift is explicitly out of scope until 4.5 lands it in the index
  encoding; the story is modes only.

### 4.20 — Partial failure exits 0

- [ ] `failed_entries` is counted, shown in the summary line and the JSON
  `finished` event — and never consulted by `main`'s exit path. A run that
  failed to read 10,000 of 100,000 files prints "10000 failed" and exits
  success. Any script or cron job wrapping xsync currently cannot tell.

**AC**: nonzero exit (distinct from usage=2 and hard-failure=1, in the spirit
of rsync's 23 "partial transfer") whenever `failed_entries > 0`; documented in
`--help` and the man page; a test locks each exit code in.

### 4.21 — A benchmark harness, because hand-rolled loops keep lying

- [ ] Three separate harness bugs corrupted measurements this cycle: the
  PowerShell "median" that returned the maximum, the zsh word-split that passed
  `--streams 2` as one token (bogus 0.03 s runs), and an unresolved hostname
  that measured three failed connects as 2.2M files/s. None were xsync bugs;
  all cost real time and one poisoned recorded numbers.

**AC**: a single harness in `benches/` that (1) **validates the transfer landed**
— file count and byte total at the destination, nonzero-exit runs discarded
loudly, (2) computes median and MAD correctly with the raw samples always
printed, (3) applies the per-platform cache drop and knows Windows has none,
(4) brackets controls (same arm first and last), and (5) emits the results as
JSON so writeups stop transcribing terminal scrollback. Every ad-hoc loop in
`benches/scripts/` migrates or dies.

### 4.22 — Phase timings are only knowable by patching the source

- [ ] Answering "exactly what is the bottleneck" (4.15) required adding
  instrumentation by hand to get scan/plan/transfer/metadata wall times. That
  breakdown plus wire-vs-logical bytes already exists internally; it just
  isn't surfaced.

**AC**: per-phase wall time in the summary under a `--timings` flag and always
in the JSON `finished` event; zero cost when off beyond reading clocks at
phase boundaries.

### 4.23 — Nobody has measured memory at 1M files

- [ ] The planner holds both trees' `FileEntry` vectors in memory, plus up to
  32 MB of loaded batch data, plus the pipeline. The 1M-file congress corpus
  ran on freya without incident, but peak RSS was never recorded on either
  end — "it didn't OOM" is the entire current knowledge.

**AC**: peak RSS measured on client and server for congress-1m on Linux and
the Mac, recorded in `docs/OS.md`; a back-of-envelope bytes-per-entry figure
derived from it; a streaming-plan story gets filed **only if** the number is
alarming. Measurement first, architecture second.

### 4.24 — Version skew wastes an afternoon; `--version` lies

- [ ] Two related hygiene failures, both hit this cycle. A stale binary on
  freya produced `version mismatch: local v1 / remote v2`, which read as a
  protocol bug until the remote's mtime was checked. Separately `build.rs`
  caches `BUILD_COMMIT`, so a fresh build after new commits reports a stale
  commit — the exact tool for diagnosing skew is itself untrustworthy.

**AC**: the mismatch error names both versions *and* both binaries' commits
and suggests the D5.2 bootstrap path to update the remote; `build.rs` re-runs
when HEAD moves (`rerun-if-changed` on `.git/HEAD` and the ref it points at);
a stale-commit repro test if practical.

### 4.44 — `wire_bytes` is not the bytes on the wire

- [ ] **Found while verifying 4.4.** `wire_bytes` counts data frames only. Every
  `encode_meta_frame` call site writes to the transport without adding to
  `report.wire_bytes`, so the figure reported in the summary line, the
  `finished` event, and every benchmark writeup understates real traffic.

Measured on congress-100k push: 1,255,326 B of metadata unreported (0.9% of
the total), rising to 9,957,243 B (1.7%) under `--no-compress`. Small here
because congress is 109,615 tiny files with 4.2× compressible payloads; a
corpus with more metadata per byte would skew further.

The practical damage is that `wire_bytes` cannot be used to evaluate any
metadata-path change — 4.4 had to build custom binaries and instrument the
encoder because the metric was blind to the exact feature it was meant to
measure.

**AC**

- Metadata frame bytes are added to `report.wire_bytes` at all four
  `encode_meta_frame` sites, in both push and pull paths.
- Either the total covers **all** bytes handed to the transport, or the
  `finished` event reports `data_wire_bytes` and `meta_wire_bytes` separately
  and the docs say which is which. Prefer the split — it keeps 4.4's
  measurement reproducible without a custom build.
- A test asserts that a transfer's reported wire bytes match the actual bytes
  written to a counting transport, so this cannot silently regress.
- `BENCHMARKv2.md` and `docs/` numbers derived from `wire_bytes` get a note
  that pre-fix figures exclude metadata.

### 4.45 — Windows preflight still stats every file

- [ ] 4.5 removed the per-file `stat` from the Unix preflight by carrying
  ownership and link count in the planning record. The Windows arm of
  `note_dropped_metadata` still calls `symlink_metadata` on every planned file,
  solely to read `FILE_ATTRIBUTE_SPARSE_FILE`.

Windows is the platform that can least afford it: its metadata operations are
the measured reason a 7900X under Windows reaches 1,099 files/s against the
same box's 6,046 on Linux.

**AC**: `file_attributes` rides in the record the way `UnixMetadata` now does
— one more optional block, or a widening of that one — with the same
round-trip test and the same before/after syscall measurement, taken on the
Windows host rather than inferred from the Unix result.

### 4.46 — Pull always asks the remote to checksum everything

- [ ] `run_client_pull` hardcodes `checksum: true` in the `SessionConfig` it
  sends (`server.rs:4380`), while `run_client_push` passes `options.checksum`.
  Every pull therefore asks the remote source to compute BLAKE3 content hashes
  for every file, whether or not the user passed `--checksum`.

Noticed while verifying 4.6; **not yet established as a bug.** The pull path
may genuinely need content identity to classify, in which case the cost is
load-bearing and the story is to document why rather than to change it.

**AC**: determine whether the flag is load-bearing. If it is, comment it at the
call site so the asymmetry with push stops looking like a typo. If it is not,
pass `options.checksum` and measure the difference on congress-100k, where
hashing 109,615 files on the far side is not free.

## Phase 9 — Parallelization research

What exists today: `--streams` parallelizes **connections**; within a single
large file, chunks stripe across those connections; between files, only files
**larger than `MAX_DATA_SEGMENT`** are distributed — small files ride the
control session single-file (see 4.14); locally, a worker pool covers
everything. 4.15 adds sender-side read/send overlap. These stories are the
axes not yet explored.

### 4.25 — Stripe small-file batches across data streams

- [ ] The multi-stream partition sends every file ≤8 MB through the control
  session. For congress that is 100% of the corpus: **`--streams N` buys zero
  parallelism on exactly the workload that is slowest**. The batches are
  already self-contained (disjoint files, own frames, own acks), so they are
  in principle distributable across the data connections the user already
  paid ~1.3 s each to open.

**Research questions**: does a second stream of batches scale small-file
throughput on Linux, or does the receiver serialize anyway (see 4.26)? Where
does it cross the connection-setup cost? Does Windows — where streams were
harmful even at the connection level — stay harmful? Interlocks with the 4.14
heuristic: if this works, the byte-share gate becomes wrong, and the heuristic
should choose streams for small-file corpora too.

### 4.26 — Receiver-side parallel apply

- [ ] The server's receive loop decodes and applies **inline on one thread**:
  verify hash, write temp, set metadata, rename, ack, next frame. During the
  4.15 investigation the server sat at ~48% CPU while receiving — and a Pi 5
  matched a 7950X, which is what a serialized apply path looks like. The
  local path already has a pool; the server path does not.

**Research questions**: decode thread feeding a bounded worker pool of
appliers — how do acks work when application is out of order (ack on verify
vs ack on commit changes crash semantics)? What does the journal require?
Measured ceiling of a single applier thread per filesystem (ext4 vs APFS vs
NTFS — NTFS's 3.5× write-cost asymmetry makes this the most Windows-relevant
performance story in the backlog). This is the other half of 4.15: sender
overlap fixes the client, this fixes the server, and the two multiply.

### 4.27 — Syscall-level batching: io_uring and its cousins

- [ ] Per small file the receiver pays open/write/fsync-free
  close/utimes/chmod/rename — six-ish syscalls, each a round trip into the
  kernel. The T1 syscall-attribution work (`benches/results/tuning/T1/`)
  already measures where that time goes. io_uring can batch and overlap them
  on Linux; macOS and Windows have no equivalent, which makes this a
  Linux-only fast path behind a runtime probe.

**Now testable on the Windows box**: the WSL2 kernel supports io_uring, so
4.47 makes this measurable on that hardware for the first time.

**Research questions**: what share of receiver wall time is syscall overhead
at 100k files (T1 data may already answer this)? Does a registered-buffers
io_uring writer beat the 4.26 thread pool, complement it, or duplicate it?
Cost: an `unsafe`-free crate exists (`io-uring` is unsafe; `tokio-uring`
pulls a runtime) — if every option needs `unsafe`, the 4.11 precedent
applies: one exemption exists, a second needs a measured win to justify it.

### 4.28 — Parallelism topologies not yet tried

- [ ] A survey story: cheap experiments, each answerable in a day, none
  worth a full story until one shows a pulse.

- **One connection, multiplexed logical streams** — N SSH connections cost
  ~1.3 s serial setup and N sshd processes; rsync-style logical channels over
  one connection would make stream count free. Overlaps 4.17's ControlMaster
  question; this is the protocol-level version.
- **Scan/transfer overlap** — the plan streams, but transfer start is gated
  on plan completeness per kind. Measure the gap between first-byte time and
  scan-complete time at 1M files; if it is seconds, pipeline planning.
- **Parallel compression** — zstd on the sender is single-threaded per batch.
  With 4.15's worker pool, compressing batch N+1 while batch N ships may come
  free; alternatively `zstd::stream` multithreading. Only matters once the
  sender is CPU-bound — re-measure after 4.15.
- **Hash parallelism** — blake3 has rayon multithreading for large inputs;
  confirm it is actually enabled on the ≥8 MB chunk path and measure whether
  it moves Manga-class transfers at all.
- **Deliberately not pursuing**: multi-process sharding (NUMA-scale problems
  we do not have) and GPU hashing (PCIe round trip dwarfs the hash).

## Phase 10 — Benchmarkability

Written 2026-08-29, after asking where this cycle's knowledge actually came
from. Two regimes exist. Regime A is the gate-able harness — `xsync-bench`
(corpus / manifest / report / gate / schedule) plus `release-bench.py`, with
rotation, paired `rsync -a` baselines, median/MAD, drift detection, and an
independent oracle. Regime B is hand-run shell loops timed with
`$EPOCHREALTIME`. Nearly everything in `BENCHMARKv2.md`, `docs/OS.md` and
`docs/STREAMS.md` — the stream sweeps, the worker curves, the OS-worth-6×
result — came from Regime B, and all three harness lies recorded in 4.21
happened there. Meanwhile every deep answer required going *around* the tool:
T1's syscall attribution needed a hand-written `LD_PRELOAD` interposer, T1.8
needed a source patch, T7.1 was blocked until `--local-workers` existed, and
phase timings had to be filed as 4.22 at all.

The definition this phase works toward: **every question the project has
actually asked this cycle should be answerable by the harness, from the
shipped binary, with no source patch, no interposer, and no hand loop.** Each
story either moves a measurement into the binary (the code), controls or
records a machine-level confounder (the operating system), or makes a
parallelism × corpus question sweepable (Phases 6, 7 and 9 are the consumers).

Prior art: the sibling project ran exactly this program for compression
(`../xc/backlog.md`, B1–B24); several stories here are its counterparts,
adapted to a tool whose runs span two machines. Relationship to existing
stories: 4.29 subsumes 4.22; 4.38 and 4.41 finish what 4.21 started; V3.33's
latency budget becomes reportable once 4.29/4.30 land; V3.40's gate policy is
wired in 4.43; T0.6's blocked cold-cache work is restated as 4.36 with a
different approach — measure residency instead of trusting eviction.

Suggested first slice: 4.29 + 4.33 + 4.34. After those, a repetition is
self-timing, self-describing, and repeatable, which upgrades every harness
cell that already exists.

**The code — 4.29 to 4.34.**

### 4.29 — An in-binary measurement core *(subsumes 4.22)*

- [ ] The engine takes **zero clock readings**: the only `Instant::now` in
  `xsync-core` is inside a test (`server.rs:7288`). The `finished` event
  carries 40+ counters and not one duration.

All timing lives in the CLI renderer, and it is measurement-hostile three
ways: the elapsed/throughput summary is printed only `if self.terminal`
(`main.rs:1334`) — piping stdout, which is what a harness does, loses it; the
clock starts on the `Planned` event (`main.rs:1351`), so scan time is
excluded from throughput; and `timestamp_unix_nanos` is stamped at render
time (`main.rs:1514`), not at event creation, so under `--progress-json` the
phase boundaries absorb the renderer's own I/O latency. The external harness
compensates with `os.wait4` — which a Windows port (4.37) cannot use.

**AC**
- Durations are carried *in* the events — per-phase and total, measured at
  the phase boundary, not reconstructed by consumers diffing render-time
  timestamps.
- `finished` carries wall, user CPU, sys CPU, and peak RSS for **both
  endpoints** — the server reports its rusage in the finish barrier, since
  half of every remote transfer's cost is invisible from the client.
  `ru_maxrss` is bytes on macOS and kilobytes on Linux; the unit trap is
  handled once, in code, with a test.
- A `--timings` flag prints the human version (4.22's AC), no longer
  TTY-gated.
- Zero measurable cost when off; the clocks at phase boundaries are the
  entire overhead.

### 4.30 — Make the phase stream tell the truth

- [ ] Phase events exist on three of four routes and are wrong on all three;
  the fourth has none.

Measured against the code: push and pull emit an **empty `metadata` phase**
(`server.rs:4173/4177`, `5068/5072`) — metadata is actually applied inside
`transfer`, so the phase that motivated the batching fix reads as free. The
local route buries the destination walk and index build inside `scan`
(`local.rs:1138–1155`), so scan throughput is not comparable to `find`. The
clone fast path emits `Started` and `Planned` *after* `transfer` opened
(`local.rs:884–914`), which is why the renderer's clock misses the clone.
And multi-stream push (`server.rs:5398+`) emits **no phase events at all** —
the last `Phase` emission in the file is line 5072 — so the route where
attribution matters most produces a stream with zero boundaries. Envelope
defects while in there: the terminal event is `"event":"finished"` but
`"type":"done"` (`main.rs:1819` vs `1659`), and `transport-selected`,
`negotiated`, `protocol-negotiated`, `filter-decision` are emitted without
`schema_version` or timestamp and are absent from `progress-json-v1.md`.

**AC**
- Every route, multi-stream included, emits the same phase vocabulary, and
  each phase covers what its name claims.
- A per-route test asserts phase coverage: T0.4's `unaccounted` convention,
  enforced — phases must account for ≥95% of wall time or the gap is named.
- `progress-json-v1.md` documents every emitted event; the `done`/`finished`
  naming is reconciled (schema bump if needed — the wire is not frozen, and
  neither is this).

### 4.31 — One definition of `wire_bytes`, and the cost of deciding to compress

- [ ] `wire_bytes` currently has four meanings: local is always 0 (nothing
  ever sets it — `local.rs:696` copies a field no code increments), push
  counts only data frames (`server.rs:3334`, `3885`, `4001`), pull counts
  every frame through the decoder (`server.rs:4709`), and multi-stream omits
  the control session entirely (`server.rs:5908`) — the session that carries
  100% of a small-file corpus.

Cross-route wire comparisons are therefore not comparisons. Related and
uninstrumented: compression is decided **per frame** by trial-compressing a
sample (`protocol.rs:583–594`, `compression.rs:39–62`), so every accepted
frame is zstd'd twice; nobody knows what that costs. The `metrics` event's
`compression_algorithm`/`compression_level` are hardcoded `None` at both
emit sites (`local.rs:857`, `1140`).

**AC**
- Bytes counted at the transport boundary, both directions, all sessions,
  all frame classes — one definition, asserted equal-by-construction across
  routes, documented in `progress-json-v1.md`.
- Split by frame class (data / metadata / ack), so metadata overhead — the
  V3.22 question — is readable from any run without an interposer.
- The double-compression cost is measured; then either the decision is
  cached per batch or the cost is recorded as acceptable, with the number.
- `--no-compress` provably produces zero compressed frames (ties to 4.4).

### 4.32 — Every knob a sweep needs, without recompiling

- [ ] The pipeline-window knee (2048) was found by recompiling; T7.1's
  recorded blocker was literally that a flag didn't exist. The tuning
  surface is still almost entirely compile-time.

Compile-time only today: `MAX_PIPELINED_FRAMES` (`server.rs:3217`) and its
**three inconsistent drain thresholds** (¾ at `server.rs:3338`, ½ at `3786`
and `5704`); `TRANSPORT_WRITE_BUFFER` (`server.rs:3225`); every `strategy.rs`
constant — where `StrategyConfig` is injectable with validation
(`strategy.rs:32–86`) and **nothing ever constructs a non-default one**; the
scanner's thread count (`scanner.rs:324` never calls `.threads()`, so the
`ignore` crate's `min(cores, 12)` default applies, unaffected by
`--local-workers`); `DEFAULT_LOCAL_QUEUE_CAPACITY = 2` (`local.rs:36`); and
the planner index budget — 64 MiB local (`planner.rs:21`) but a *different*
hardcoded 32 MiB on the multi-stream path (`server.rs:5528`), so the two
routes spill at different tree sizes. `SessionConfig` re-states
`batch_bytes`/`chunk_bytes` as bare literals (`server.rs:3494`), so even a
recompile can half-apply.

**AC**
- A hidden `--tune name=value` (repeatable) covering a documented allowlist
  of the constants above. Unknown names are refused, not ignored.
- Every tuned value is echoed in the `finished` event — a result that
  doesn't record its knobs is scrollback.
- The duplicated literals are routed through one constants table first, so a
  tune cannot apply to one code path and not another.
- Defaults bit-identical when the flag is absent; this is a measurement
  surface, not a config surface, and the help text says so.

### 4.33 — Report what the run did, not what was asked

- [ ] `--streams 8` on a pull runs 1 stream and **reports 8**:
  `sync_pull_server` (`server.rs:6346`) never consults `options.streams`,
  yet `Started` (`server.rs:4267`) and `Finished` (`5120`) echo the
  requested value. The local route does the same ("retained only for event
  reporting", `local.rs:261`). A streams sweep over pull would produce N
  identical arms with N different labels — the exact shape of lie that 4.21
  exists to prevent, emitted by the tool itself.

Same family: `queue_high_water` is capped by the 1024-slot channel
(`scanner.rs:24`), so it saturates on any tree over ~1k entries and stops
discriminating; `DispatchStats` (`strategy.rs:121`) is collected and has
zero consumers — dead instrumentation that looks like coverage.

**AC**
- `finished` reports **effective** values — streams actually used, workers
  actually spawned, scanner threads — alongside requested ones; requesting
  an option a route ignores produces a warning event.
- `DispatchStats` is surfaced in `finished` or deleted.
- The harness cross-checks label against effective values and voids
  mismatched cells, so this class of error dies at both ends.

### 4.34 — Hermetic runs: make the reset expressible

- [ ] A clean-slate second run currently requires out-of-band surgery
  against paths the tool never prints: `rm ${TMPDIR}/xsync-resume-*`
  (journal, keyed by `blake3(src\0dest)` — `journal.rs:107` — so repeated
  runs of the same command *share* state, and an interrupted rep silently
  contaminates the next one's `resumed_bytes`); `rm ${TMPDIR}/.xsync-planner-*`
  (spill files, `planner.rs:25`); and deleting `hashes.redb` under
  `XDG_CACHE_HOME` with a `HOME` fallback (`hash_cache.rs:184`) — there is
  no `--no-hash-cache`, so run 1 hashes everything and run 2 hashes nothing,
  worth seconds (`hash_cache.rs:15–20`).

Also per-run destination pollution: the reflink probe writes and deletes two
files at the destination root every run (`clone.rs:372`), and on Linux it
spawns `cp --reflink=always` to do it (`clone.rs:391`).

**AC**
- The binary can print its state paths (journal dir, planner spill dir, hash
  cache file) — `--print-state-paths` or equivalent.
- `--no-hash-cache` and `--fresh` (ignore any existing journal) exist and
  are honored on every route.
- `benches/README.md` documents the full reset recipe; the harness performs
  it between arms.
- A locked-in test: two identical runs after a reset report
  `resumed_bytes = 0` and identical checksum-cache miss counts.

**The operating system — 4.35 to 4.38.**

### 4.35 — Standing OS counters, replacing the interposer one-offs

- [ ] Answering "how many syscalls per file" took a hand-written
  `LD_PRELOAD` interposer because SIP blocks `dtruss` even with Full Disk
  Access, plus non-default build flags for the backtraces (T1.6). The
  finding — 25.1 → 14.1 syscalls/entry, against rsync's 6.4 — is the most
  consequential number in the tuning program, and the T1.3 budget (unmet at
  0.515 paired ratio) has **no standing measurement**: nobody notices if it
  regresses.

The cheap standing sources were never wired up: `getrusage` gives faults and
voluntary/involuntary context switches beyond what 4.29 takes; Linux
`/proc/self/io` gives `syscr`/`syscw`/`read_bytes`/`write_bytes` — a
per-run syscall-rate proxy for free; macOS has `proc_pid_rusage`.

**AC**
- A per-endpoint OS-counter block in `finished`: faults, context switches,
  and on Linux the `/proc/self/io` set; per-entry rates derived in the
  report.
- Per-platform availability documented; an absent counter is reported
  absent, never zero — macOS has no `syscr`, and the report must not
  pretend otherwise.
- The T1.3 syscall budget becomes a gate-able assertion on Linux
  (`(syscr+syscw)/entry` against a stated bound), so the interposer is
  needed for *attribution* only, never for *detection*.

### 4.36 — Cache state as a measured fact, not a label

- [ ] The schema constrains cache labels to
  `first_pass`/`warm`/`cold_evicted`, but eviction is *trusted*, not
  verified: `purge` fails with `Operation not permitted` in this checkout,
  `drop_caches` needs root freya doesn't grant, every Windows number ever
  recorded is warm, and SSH cold runs are **refused** because the remote's
  cache state is unknowable (T0.6, blocked). The whole warm/cold discipline
  rests on assuming a command worked.

Measure residency instead: `mincore(2)` exists on both Linux and macOS. A
bounded sample of the corpus's pages, probed immediately before the rep,
turns "we ran purge" into "0.4% of source pages were resident".

**AC**
- A residency probe (in `xsync-bench`, not the binary) samples the corpus
  and records **percent-resident** with every repetition; cache labels are
  *derived* from the measurement, with thresholds stated in the schema.
- Remote residency runs through the staged `xs` agent — it is already on
  every bench host via `ensure-linux-agent.sh` — which unblocks honestly
  labeled SSH cold runs, currently refused by design.
- Where no probe exists (Windows), the label says `inferred` and the
  corpus-larger-than-RAM strategy is the documented cold path (4.10's AC).
- Probe cost is bounded, measured, and excluded from the timed region.

### 4.37 — Port the gate-able harness to Windows

- [ ] The platform with the headline finding — 5.8× slower than Linux on
  identical silicon — is the one platform the harness cannot run on.
  `release-bench.py` is POSIX to the bone: `os.wait4`, `pgrep`, `df`,
  `purge`/`drop_caches`. Every Windows figure in `BENCHMARKv2.md` was
  produced by hand-rolled PowerShell, which is how the median-that-was-
  actually-the-maximum happened (4.21).

4.29 shrinks this port deliberately: once wall/CPU/RSS come from inside the
binary on both ends, the harness needs only orchestration, staging, oracle
verification, and reporting — all portable Python plus `xsync-bench`, which
is Rust and already compiles for Windows.

**AC**
- The full pipeline — corpus staging, rotation, oracle verify, report — runs
  on the Windows box for congress-100k, using 4.29's in-binary accounting
  (or Job Objects where external accounting is still needed).
- Cache labels honest per 4.36: warm-only until a residency story exists,
  and the report says so.
- The 4.12 Defender A/B is expressible as two harness arms, so that story
  stops waiting on hand-run sessions.
- The PowerShell loops die, the same way 4.21 killed the zsh ones.

### 4.38 — A watchdog for the confounders the harness has already met

- [ ] Every voided session this cycle was voided by the *machine*, not the
  tool: 641 GiB written to a 256 GB drive killed the first Manga sweep and
  its baselines (4.1); the Kingston SLC cliff produced a 1.6× drift caught
  only because a bracket arm happened to run; the Pi's thermal drift (18.13
  → 27.11 s) invalidated its streams numbers; and enclosure temperature
  cannot even be logged, because `smartctl` can't pass SMART through the USB
  bridge. 4.1's redesign prescribes budgets, brackets, and between-arm `dd`
  checks — as *operator discipline*. Discipline doesn't survive contact
  with a long session; harness features do.

**AC**
- A session declares its target device and capacity; the harness tracks
  cumulative bytes written and **refuses to start an arm** that would
  exceed the declared budget, rather than warning afterwards.
- Between-arm device probe: a bounded `dd` to scratch, compared against the
  session's opening baseline; degradation past a threshold pauses the
  session instead of poisoning the next arm.
- Bracket arms (identical first and last) are scheduled automatically,
  their drift computed, and the session stamped **voided** past a stated
  threshold — the 4.1 postmortem, as a feature.
- Thermal and frequency state logged where readable (Linux
  `/sys/class/thermal` and cpufreq; macOS thermal pressure); where not
  readable, the absence is recorded, per the V3.21 precedent of settling
  thermal questions by design when they can't be settled by measurement.

**Parallelization × corpus — 4.39 to 4.43.**

### 4.39 — Corpus shape vectors: heuristics as functions of shape, not names

- [ ] Every corpus-dependent conclusion is currently indexed by a *name*
  (congress, cb7, Manga) instead of by the properties that caused it. The
  4.14 streams rule needs share-of-bytes above `MAX_DATA_SEGMENT`; V3.20
  showed core count fails to predict the worker optimum; cb7 only earned
  its place because someone noticed it is 82.6% small by count but 68%
  large by bytes. And shape governs what an experiment *can* test:
  congress-10k has 11,288 directories for 11,280 files, which silently made
  T7's directory-affine dispatch untestable. The registry pins digests and
  file counts — never shape.

**AC**
- `xsync-bench corpus describe` emits a shape manifest: entries by kind,
  total bytes, size percentiles (p50/p90/p99/max), **share of bytes above
  8 MiB** (the 4.14 signal), directories-per-file, depth distribution,
  symlink count, and sampled compressibility (bounded sample, zstd-3, the
  same bucket scheme as `compression::decide`).
- Stored beside the pinned digests, embedded in every report — a result
  names its workload's shape, not just its name.
- 4.14's stream heuristic and V3.20's worker probe take shape-vector
  inputs, so their rules are stated as thresholds on measured properties
  and are testable on any future corpus, including 4.16's.

### 4.40 — Synthetic twins for the private corpora

- [ ] No third party can reproduce a single real-corpus cell. `corpora/` is
  gitignored and machine-pinned: Manga is copyrighted media, cb7 is a
  personal project tree, congress is a local copy of *public* govinfo/
  voteview data with no provisioning script, and docker-raw is registered
  in `real_corpora()` with **no pinned digest and an empty directory** —
  Corpus D is documented in `TUNING.md` and cannot actually run.

`xsync-bench corpus` already generates seven deterministic seeded classes.
What's missing is the bridge: generators parameterized by 4.39's shape
vectors, so `cb7-twin` has cb7's measured size distribution, depth, and
compressibility rather than a guess.

**AC**
- `congress-twin`, `cb7-twin`, `manga-twin`: deterministic from a seed,
  fitted from the shape manifests, digest-pinned in the registry exactly
  like the real corpora.
- A same-host calibration run records the twin-vs-real delta per headline
  metric. A conclusion that holds on the real corpus and not its twin is
  flagged — the delta is itself a finding about *which shape features
  matter*, which is 4.16's question asked cheaply.
- congress additionally gets a provisioning script from the public source,
  since it needs no twin at all.
- docker-raw is either provisioned and pinned, or removed from the
  registry; a registered corpus that cannot run is a trap.

### 4.41 — A parallelism sweep is a harness mode, not a shell loop

- [ ] Every parallelism sweep this cycle — workers, streams, pipeline
  window — was a hand loop, and all three of 4.21's recorded harness lies
  happened inside hand loops. `release-bench.py` sweeps *methods and
  routes*; nothing sweeps *knob values*. The axes the research program
  needs are exactly `--local-workers` × `--streams` × `--compress-level` ×
  the 4.32 tune surface × corpus.

**AC**
- Sweeps are declared as axes; the harness prints run count, estimated
  duration, and **estimated bytes written** before starting, checked
  against 4.38's device budget — the first Manga sweep would have been
  refused at the plan stage.
- Arms are interleaved/rotated through the existing `schedule` machinery
  (which already rejects orderings that never cross over), so thermal
  drift spreads across configs instead of penalizing whatever ran last.
- Every cell inherits the full Regime-A policy: paired baseline, ≥5 reps,
  median/MAD, oracle verification, drift detection.
- Axes a route ignores are refused up front (per 4.33's effective-value
  reporting) — no burning 5 reps × N arms on a pull-streams sweep that
  runs the same configuration N times.
- Output is the existing `xsync.bench.report.v1` schema, so `report`,
  `gate`, and the comparison tooling read sweep results unchanged.

### 4.42 — Regime maps: RTT and bandwidth as swept variables

- [ ] The pipeline-window knee (2048, chosen in 4.15) was measured at
  exactly one RTT — the 5.3 ms in-session figure — and 4.18 already flags
  that it may be wrong at WiFi RTTs. T8.1's compression crossover table is
  blocked entirely. Both are blocked on the same thing: link shaping is
  macOS-only `dnctl`, three fixed rates, ssh-route-only — and fails with
  `Operation not permitted` anyway.

Linux `tc`/netem does both delay and rate, is available with root on hosts
the project already controls (mars, freya), and shapes *any* route.

**AC**
- netem shaping on a Linux pair with RTT (≈1/5/15/30/60 ms) and bandwidth
  (50/100/1000 Mbit) as sweep axes under 4.41.
- Shaping is **verified, not trusted**: measured RTT and throughput through
  the shaped path recorded before each arm — the same principle as 4.36.
- The pipeline-window knee re-measured as a function of RTT via 4.32's tune
  surface; the drain rule confirmed, or replaced with an RTT-aware one and
  the change recorded in `docs/STREAMS.md`.
- T8.1's crossover table — the link speed below which compression wins —
  produced for at least congress and Manga, closing the oldest blocked
  T-task.

### 4.43 — Wire the gate: a baseline in the tree, a job in CI

- [ ] `xsync-bench gate` is fully implemented — ≥5 reps, 15% dispersion
  ceiling, 15% tolerance, paired ratios only, environment/digest matching —
  and is invoked by **zero scripts and zero workflows**. No baseline report
  is checked in to gate against. `tasks.md:1511` lists "benchmark strict
  gate all pass" as a release criterion that nothing enforces. This is
  V3.40, reduced from design work to wiring.

**AC**
- A CI job runs one small synthetic-corpus cell (sized to CI time) on a
  pinned runner class and gates on paired ratios, per the existing policy:
  noisy rows neither pass nor fail, and say so.
- A baseline report is nominated, checked in with its environment block,
  and the comparison refuses across mismatched environments — the gate's
  existing rule, finally exercised.
- Thresholds are set for large regressions only, with the shared-runner
  noise caveat documented rather than implied.
- The release checklist item points at the workflow that enforces it.

## Phase 11 — WSL, and what identical hardware can settle

Written 2026-08-30. The Windows box (7900X) has WSL2 enabled with no distro
installed, on build 26200 — new enough for mirrored networking. Installing one
turns that machine into the only host in the fleet that can run three operating
system paths over **the same CPU, the same NVMe, and the same network link**.

Every OS conclusion recorded so far changed more than one variable at a time.
`docs/OS.md` says the OS is worth ~6× and the CPU roughly nothing — the 7900X
does 1,099 files/s under Windows against 6,046 under Linux — but that pair also
crossed filesystems, kernels, and machines. These stories exist to hold the
hardware still.

### 4.47 — Stand WSL2 up as a measurement platform

- [ ] Install a distro on the Windows host and make it reachable, reproducible,
  and able to build `xs`.

**Setup** (the distro install prompts for a UNIX account, so it is an operator
step):

```
wsl --install -d Ubuntu
```

`C:\Users\sanjee\.wslconfig`:

```
[wsl2]
networkingMode=mirrored
memory=16GB
processors=16
swap=0
vmIdleTimeout=-1

[experimental]
autoMemoryReclaim=disabled
hostAddressLoopback=true
```

`autoMemoryReclaim` belongs under `[experimental]`, not `[wsl2]` — WSL logs
`Unknown key 'wsl2.autoMemoryReclaim'` and carries on, so a typo here is silent
except for one line at boot. `hostAddressLoopback` lets the host reach the VM by
its own address.

`mirrored` gives WSL the host's IP, so it is reachable at `192.168.1.120`
without `netsh portproxy`. The rest is not convenience: **dynamic memory
reclaim and idle-VM suspension are exactly the class of drift that produced the
three harness lies in 4.21**, and a benchmark host that resizes itself between
runs cannot be trusted.

Inside the distro: `systemd=true` in `/etc/wsl.conf`, `openssh-server` on
**port 2222** (the Windows sshd owns 22 and mirrored mode shares the port
space), `build-essential`, and rustup. Then `wsl --shutdown` so systemd and the config
take effect.

**Key authorisation is not where it looks.** There is no
`C:\Users\sanjee\.ssh\authorized_keys` on this host. `sanjee` is an
administrator, and the Windows sshd config ends with

```
Match Group administrators
       AuthorizedKeysFile __PROGRAMDATA__/ssh/administrators_authorized_keys
```

so the accepted key lives in `C:\ProgramData\ssh\administrators_authorized_keys`.
Copying it across from WSL is unreliable — that file's ACL grants only SYSTEM
and Administrators, and a WSL session runs under the *unelevated* token, which
UAC strips of administrator group membership. Write the key in directly
instead:

```
mkdir -p ~/.ssh && chmod 700 ~/.ssh
cat >> ~/.ssh/authorized_keys <<'KEY'
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINh14bwS7GKQcz7VpmHU18DKIV9JxjjvlvP4iIPdzobx sanjee@Sanjees-MacBook-Pro.local
KEY
chmod 600 ~/.ssh/authorized_keys
```

WSL's sshd is an ordinary Linux sshd, so the per-user path applies there and no
`Match Group` block is involved.

**Two blockers that are not obvious from the logs.** Both were hit on the first
attempt, with sshd reporting `active` and listening on `0.0.0.0:2222` the whole
time.

*`wsl --shutdown` leaves nothing running.* The VM does not come back until
something launches it, so sshd is not merely unreachable, it does not exist.
Start it with `wsl` after any shutdown. Running `wsl.exe` over SSH also needs
`--cd /`: it tries to translate the session's working directory and fails with
"The system cannot find the path specified".

*WSL's Hyper-V firewall blocks inbound by default.* With mirrored networking the
VM holds the host's address (`eth0` shows `192.168.1.120/24`), but
`Get-NetFirewallHyperVVMSetting` reports `DefaultInboundAction: Block` for VM
`{40E0AC32-46A5-438A-A0B2-2B479E8F2E90}`, so even `127.0.0.1:2222` from the host
itself refuses. The pre-existing "OpenSSH Server (sshd)" allow rule covers port
22 only. Elevated, and scoped to the one port rather than flipping the default
inbound action, which would expose every port in the VM:

```
New-NetFirewallHyperVRule -Name "WSL-SSH-2222" -DisplayName "WSL SSH 2222" `
  -Direction Inbound -VMCreatorId '{40E0AC32-46A5-438A-A0B2-2B479E8F2E90}' `
  -Protocol TCP -LocalPorts 2222 -Action Allow
New-NetFirewallRule -DisplayName "WSL SSH 2222" -Direction Inbound `
  -Protocol TCP -LocalPort 2222 -Action Allow
```

The first admits traffic to the VM, the second admits it to the host from the
LAN. Both are firewall changes and belong to the operator.

**AC**

- `ssh -p 2222 sanjee@192.168.1.120` works from the Mac, and `xs` builds there.
- The `.wslconfig` pinning is committed to `docs/` with the reasoning, not just
  applied — a future run on an unpinned VM is not comparable.
- Recorded honestly: `drop_caches` works inside WSL, but Windows still caches
  the VHDX underneath, so "cold" is colder than warm and not as cold as freya.
  Any writeup says so rather than implying parity.

**Operational finding to record while it is fresh.** Building on the Windows
host over SSH fails with `os error 448`, "the path cannot be traversed because
it contains an untrusted mount point". Every tool in `.cargo\bin` is a symlink
to `rustup.exe`, and Windows does not evaluate symlinks for a remote session by
default. Invoking the real toolchain binaries directly works and needs no
system change:

```
$tc = "C:\Users\sanjee\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin"
$env:RUSTC = "$tc\rustc.exe"; & "$tc\cargo.exe" build --release
```

Setting `RUSTC` matters as much as calling `cargo.exe` by path: cargo finds
`rustc` on `PATH` and hits the same symlink. The alternative,
`fsutil behavior set SymlinkEvaluation R2L:1`, is a system security setting and
belongs to the operator.

### 4.48 — Decompose the OS penalty on identical hardware

- [ ] Answer the question `docs/OS.md` raises and cannot settle: **is Windows
  slow because of NTFS metadata, or the Win32 layer, or both?**

WSL supplies three paths that differ in exactly one thing at a time:

| path | userspace | filesystem | isolates |
|---|---|---|---|
| native Windows | Win32 | NTFS | today's baseline |
| WSL `~/` | Linux | ext4 in a VHDX on the same NVMe | Linux syscalls + Linux FS |
| WSL `/mnt/c/` | Linux | NTFS via drvfs | Linux syscalls, Windows FS |

**The third row is the decisive one.** If WSL-ext4 lands near native Linux and
WSL-drvfs near native Windows, the penalty is the filesystem. If both WSL paths
are fast, it is the Win32 layer. If drvfs is worse than native Windows, the
bridge dominates and the row says nothing about NTFS — which is itself worth
knowing before anyone quotes it.

**AC**

- congress-100k over SSH to all three paths, plus the existing native-Windows
  and native-Linux numbers, medians with MAD, bracketed controls.
- cb7 as a second shape, since congress is one file per directory and cb7 is
  7.2 — directory-metadata cost is the suspected mechanism and the two corpora
  disagree on it.
- `docs/OS.md` gains the decomposition and either keeps or retires "the OS is
  worth ~6×" with the mechanism named.
- drvfs is reported as the bridge it is, not as "NTFS from Linux".

**Consumers.** 4.26 (receiver-side parallel apply) wants NTFS-vs-ext4 write cost
on one disk. 4.27 (io_uring) becomes testable on this hardware for the first
time, since the WSL2 kernel supports it. 4.12 (Defender) gets a cleaner
isolation, as Defender treats the VHDX and `/mnt/c` differently. 4.18 fills a
matrix cell.

### 4.49 — Re-measure 4.15 and the freya baseline on an idle host

- [ ] The 4.15 A/B was run while freya was executing an unrelated job at ~950%
  CPU. Five paired runs agreed, so the **1.86× ratio is reproducible**, but the
  absolute figures are not the platform's: equivalent code measured 14.4 s on an
  idle freya earlier in the cycle against 24.88 s under load.

Until this is redone, **1.86× must not be quoted as the platform number** — the
honest statement today is "1.86× under contention, 1.15× on an idle Windows
receiver, unmeasured idle on Linux".

**AC**

- congress-100k, Mac → freya, both arms, alternating, with the host confirmed
  idle (`uptime` and top process recorded alongside the numbers).
- If the idle ratio differs materially from 1.86×, `BENCHMARKv2.md` and the
  4.15 entry are corrected rather than annotated.
- Load state is recorded for every future cross-host benchmark, since this is
  the second measurement this cycle contaminated by something outside the tool.

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
