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
| Measurement discipline | 4.15 is **2.05–2.15×** (4.49, three sessions). freya is stable across sessions: the same code measured 24.88 / 24.79 / 26.30 s, a 6% spread. The lone 14.4 s figure reproduces under no configuration and is treated as erroneous. Every benchmark verifies exit status and landed file count |

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

### 4.12 — The Windows Defender tax *(measured 2026-08-30)*

- [x] congress-100k, one NTFS volume, two sibling directories so both arms ran
  without touching Defender again: `C:\xsdef` scanned, `C:\xsab` covered by a
  temporary `Add-MpPreference -ExclusionPath`. Real-time protection enabled
  throughout. Alternating arms, three rounds.

| arm | reps | median | files/s |
|---|---|---|---|
| Windows as configured | 87.23, 87.18, 87.48 | **87.23 s** | 1,257 |
| Windows, benchmark path excluded | 72.06, 71.19, 71.39 | **71.39 s** | 1,536 |
| WSL2 ext4, same NVMe, verified | 18.70, 18.34, 18.70 | **18.70 s** | 5,862 |

**Defender costs 1.22× — 15.84 s, or 144 µs per file.** That is 18% of the wall
clock a Windows user actually experiences.

**But it is not the explanation for Windows being slow.** Against the Linux
reference on the same NVMe the gap is 68.53 s, and Defender is 15.84 s of it.
**Scanning accounts for 23% of the Windows-versus-Linux gap; the other 77% is
Windows itself.** With scanning entirely removed Windows still runs **3.82×**
slower than Linux on identical hardware.

**The cost is per-file, not per-byte — confirmed directly.** On 3.94 GiB in 7
large files, scanned and excluded are indistinguishable (62.65 s against
65.98 s, the difference within noise and in the wrong direction). 144 µs across
7 files is a millisecond. Defender is a tax on *file creations*, so it scales
with file count and vanishes on bulk data. The hypothesis this story was filed on —
that real-time scanning was the leading explanation for 1,025 files/s against
6,351 — is therefore **false**, and worth stating plainly because it was the
prevailing assumption.

**Reporting**: the headline is "Windows as configured", 1,257 files/s, since that
is what users have. The excluded figure is explanatory only.

**Operator action outstanding**: the temporary exclusion on `C:\xsab` is still in
place and should be removed. The pre-existing stale exclusions pointing at
`C:\Users\sanje\...` (the pre-local-account profile) are unrelated and still
worth cleaning up.

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

### 4.17 — SSH transport: how much is the tunnel costing? *(measured 2026-08-30 — nothing)*

- [x] Decomposed on a Linux pair (freya ↔ orion, 1 GbE), then re-run
  same-session at matched transfer size after the first pass produced a
  contradiction. **SSH costs nothing measurable, and no sender optimisation
  beats it.**

**All senders, 2 GB, freya → orion, same session:**

| sender | throughput |
|---|---:|
| C++ `send()`, 4 MB writes | **78.1 MB/s** |
| C++ `send()`, 256 KB writes | 78.1 MB/s |
| Rust `write_all`, 4 MB | 77.6 MB/s |
| C++ + 8 MB `SO_SNDBUF` | 77.4 MB/s |
| **ssh → `cat >/dev/null`** | **77.4 / 75.9 MB/s** |
| C++ `splice()` pipe→socket | 75.9 MB/s |
| C++ `MSG_ZEROCOPY` | failed, `ENOBUFS` |

**Everything lands in a 3% band.** Language makes no difference, write size makes
no difference from 256 KB to 4 MB, a larger `SO_SNDBUF` makes no difference,
zero-copy `splice()` is marginally *worse*, and `MSG_ZEROCOPY` needs a locked-
memory limit we have not raised. **SSH sits inside the same band as raw
sockets.**

**A number to retract.** The first pass had ssh at **84–85 MB/s**, *above* raw
TCP, and the write-up reasoned about why OpenSSH might beat a naive loop. It
does not. That measurement used 1 GB transfers while the raw baselines used
2 GB; at matched size and in the same session ssh gives 75.9–77.4. **The
comparison was never like-for-like.** A result that says "the encrypted path
beats the unencrypted one" should have been treated as a measurement bug
immediately rather than explained.

**The link is the limit, and it is symmetric.**

| direction | raw TCP |
|---|---:|
| freya → orion | 77.6 MB/s |
| orion → freya | 74.1 MB/s |

~75 MB/s both ways, or **60% of gigabit line rate**, with the NIC reporting
zero errors, drops, carrier faults or collisions. That is what this path
delivers.

**Consequences.**

- **xsync is at the wire.** It moves 75.8 MB/s of logical data on cb7 against a
  ~77 MB/s path. There is essentially nothing left to win on this pair.
- **Cipher selection is dead**: 4% across four ciphers, and AES-GCM ties
  ChaCha20 even with a Pi 5 on one end (Cortex-A76 has ARMv8 crypto).
- **QUIC or TLS cannot be justified on throughput.** They would have to beat a
  transport already indistinguishable from raw sockets. Phase 12 must rest on
  the daemon, trust model and connection reuse — its stated premise needs
  rewriting.
- **Only a faster link moves this**, which is 4.50. Worth noting the path gives
  60% of line rate, so the first question there is whether that is the switch,
  the cabling or the Pi, not simply "buy 2.5 GbE".

**Still open**: the 5.3 ms in-session round trip and 195 ms session setup, both
measured on the macOS pair through a USB adapter. Latency, not throughput; 4.50
retests them.

**Four wrong "raw TCP" numbers preceded the right one**: `nc` absent, a Python
sink capping at 71.6, `socat` at an 8 KB default block, and `socat` at 1 MB
still short. Each read as a link measurement and was a tooling measurement.

### 4.18 — Platform and hardware matrix: where is the map blank?

- [ ] Current coverage is deep but narrow: macOS on Apple Silicon, Linux on two
  Zen 4 desktops and a Pi 5, Windows on one Zen 4 desktop, all on a ~1 GbE LAN.
  The OS conclusions in `docs/OS.md` (OS worth ~6×, CPU ~0%) are strong on this
  hardware and untested off it.

Blank areas, in priority order:

1. **Faster links** — at 2.5/10 GbE does the sender bottleneck (4.15) simply
   dominate everything, or do new ceilings appear? Cheapest decisive test of
   whether tuning generalizes.

   *Hardware survey, 2026-08-30.* Measured on the current pair:

   | transport | throughput |
   |---|---:|
   | 1 GbE, Mac USB adapter, `dd \| ssh cat` | 86.5 MB/s |
   | Wi-Fi 6, 5 GHz 80 MHz, same test | 72.3 MB/s |
   | xsync, large files | 64.9 MB/s |

   Wi-Fi is only 16% behind the wired USB gigabit adapter, which says the
   adapter — not the medium — is the weak link.

   **Thunderbolt/USB4 peer-to-peer is not available on this pair.** The Mac has
   Thunderbolt Bridge (`bridge0`, ports `en1`–`en3`, 40 Gb/s buses). The
   Windows box (Gigabyte X870) exposes `USB4 Root Router` and
   `USB4(TM) Host Router (Microsoft)` but has **no networking device** in the
   `Net` class for it: IP-over-Thunderbolt on Windows is an Intel-driver
   feature, and the generic Microsoft USB4 stack on AMD does not provide it. A
   USB-C cable between them enumerates but yields no IP link.

   **The cheap path is already half-installed.** The Windows box has a *Realtek
   PCIe 2.5GbE* controller, currently auto-negotiated down to 1 Gbps because
   the other end is gigabit. A USB-C 2.5GbE adapter for the Mac plus a direct
   cable and static addresses gives **2.5 Gbps (~280 MB/s) point-to-point with
   no switch** — about 3.3× today's SSH ceiling, which is enough headroom to
   reveal whether SSH crypto or xsync's chunked path binds next.

   **10 GbE is premature.** It needs a PCIe NIC plus a Thunderbolt adapter, and
   with `ssh` topping out at 86.5 MB/s it would mostly measure OpenSSH until
   4.17's transport work lands.
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

### 4.19 — A `chmod` never syncs *(fixed 2026-08-30)*

- [x] `metadata_matches` compared kind, size and mtime, never mode. A `chmod` on
  the source changed nothing a content comparison could see, so the destination
  kept the old permissions forever and the run cheerfully reported "1 skipped".

**Design.** The planner gained a `Difference` verdict — `None` / `ModeOnly` /
`Content` — and a `Classification::MetadataOnly` feeding a new
`EntryPlan::metadata` bucket. Mode-only drift is repaired with
`MetadataOperation::SetFile`, which already existed on the wire and now carries
the mode as well as the mtime, so **no content is retransferred**.

**The trap this had to avoid.** `permission_mode` invents `0o755`/`0o644` on
hosts without Unix permissions. Comparing an invented mode against a real one
would classify every file as permanently drifted and re-chmod the whole tree on
every run — worse than the bug. A new `CAP_UNIX_MODES` capability gates the
comparison: modes are compared only when the local host is Unix **and** the peer
advertises real modes.

**Verified**

| case | result |
|---|---|
| local → local, file and directory | `600` / `700` applied, **0 bytes transferred**, 2 mode-repaired |
| Mac → Linux over SSH | `600` / `700` applied, 0 bytes, 1 mode-repaired |
| third run, both routes | clean no-op, 0 repaired |
| Mac → **Windows**, three consecutive pushes | 0 mode-repaired every time — **no churn from synthesized modes** |

Four planner tests pin the classification, including the synthesized-mode
exemption, symlinks (whose permission bits are not portably settable, so drift
there is unrepairable and reporting it would be noise), and the rule that a
content change outranks a mode change.

**Reported, not silent**: the summary line and the JSON `finished` event carry
`metadata_repaired`.

**Noticed in passing, not fixed.** Directory *sizes* differ across platforms
(macOS reports 96 where Linux reports 4096), so on a cross-platform transfer
directories always classify as `changed` rather than `unchanged`. Harmless today
— the directory metadata sweep runs over every directory regardless — but it
means the mode-repair count under-reports directories on cross-platform runs,
and any future work that trusts `directories.unchanged` should know.

### 4.20 — Partial failure exits 0 *(already fixed; verified 2026-08-30)*

- [x] Stale. `main` returns `RunOutcome::Partial` when `report.partial_failure()`
  is set, which maps to **exit 23** — rsync's partial-transfer convention. A run
  with one unreadable source file prints `… 1 failed, partial failure` and exits
  23, while a clean run exits 0.

Filed on a reading of the code that was already out of date. Kept as a closed
entry rather than deleted, because "the exit code lies" is the sort of claim
that gets repeated from notes.

**AC**: nonzero exit (distinct from usage=2 and hard-failure=1, in the spirit
of rsync's 23 "partial transfer") whenever `failed_entries > 0`; documented in
`--help` and the man page; a test locks each exit code in.

### 4.21 — A benchmark harness, because hand-rolled loops keep lying

- [ ] Three separate harness bugs corrupted measurements this cycle: the
  PowerShell "median" that returned the maximum, the zsh word-split that passed
  `--streams 2` as one token (bogus 0.03 s runs), and an unresolved hostname
  that measured three failed connects as 2.2M files/s. None were xsync bugs;
  all cost real time and one poisoned recorded numbers.

**A third failure mode, added 2026-08-30 after it produced a retracted
headline.** Running `xs ... -q >/dev/null 2>&1` without checking the exit status
turns every failure into a plausible timing. On WSL this yielded a steady
7.85 s "result" across eight runs that had all failed, and it survived until
513 MB/s over a gigabit link made it obvious. The harness must treat a non-zero
exit and a destination file count that does not match the corpus as **hard
errors that discard the run**, never as data.

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

- [x] Two related hygiene failures, both hit this cycle. A stale binary on
  freya produced `version mismatch: local v1 / remote v2`, which read as a
  protocol bug until the remote's mtime was checked. Separately `build.rs`
  caches `BUILD_COMMIT`, so a fresh build after new commits reports a stale
  commit — the exact tool for diagnosing skew is itself untrustworthy.

**AC**: the mismatch error names both versions *and* both binaries' commits
and suggests the D5.2 bootstrap path to update the remote; `build.rs` re-runs
when HEAD moves (`rerun-if-changed` on `.git/HEAD` and the ref it points at);
a stale-commit repro test if practical.

**Delivered.** The stamp was wrong for a precise reason: `build.rs` already
watched `.git/HEAD`, but on a branch that file holds `ref: refs/heads/main` and
never changes as commits land. Every commit after the first went unnoticed. It
now follows `HEAD` to the ref it names and watches that, plus `packed-refs`
(where the ref lives once packed, when the loose file does not exist) and
`index`. `.git` is also resolved when it is a *file*, so a worktree build does
not silently watch nothing. Verified by committing and rebuilding without
touching `build.rs`: the stamp moved `6bfeafb0d893-dirty` → `da63c3f14c91`.

The commit is now exact. **The `-dirty` marker remains best-effort** — an
uncommitted edit that never reaches the index does not re-trigger the script,
so a binary can report clean while holding modified source. Making it exact
would require re-running `build.rs` on every build, which is a real incremental
cost for a marker that is a hint, not a guarantee.

For the mismatch error, the remote's commit is **not** available where the
error is raised: the mismatch comes from a frame header, before any exchange
that could carry it. Rather than probing the remote on the failure path — where
it may already be unreachable, and a hanging diagnostic is worse than none —
the client prints its own commit and the exact commands to check and update the
other end. Carrying build identity in the handshake, so the error can name both
directly, belongs with the version-skew work in `backlog-release.md` R1.4.

### 4.44 — `wire_bytes` is not the bytes on the wire

- [x] **Found while verifying 4.4.** `wire_bytes` counts data frames only. Every
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

**Delivered, but not the way the AC framed it.** Adding `+=` at the four
`encode_meta_frame` sites is what the AC asked for and is exactly the design
that produced the bug: a hand-maintained sum drifts the moment someone adds a
write site. The count is instead taken at the **transport boundary**, by
`CountingWriter`/`CountingReader` in `transport.rs`, which cannot drift because
it does not know about message types. Push counts writes; pull counts reads,
because that is the direction its payload travels.

`wire_bytes` is now the exact total. The old per-frame sum is kept as
`data_wire_bytes`, and the JSON reports `data_wire_bytes` and `meta_wire_bytes`
alongside it, so 4.4's measurement is reproducible without a custom build.

`data_wire_bytes` is **zero when the backend does not distinguish** — the rsync
fallback, and local copies with no wire at all. A consumer computing a metadata
share has to check for that, or it reads "all overhead" where the honest answer
is "not measured".

Measured on a 720-file corpus of small compressible files: 76,633 wire bytes,
of which **9,715 (12.7%) were previously unreported**. Far above the 0.9% seen
on congress-100k, because metadata share rises as payloads shrink and compress.

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

### 4.25 — Stripe small-file batches across data streams *(done 2026-08-30)*

- [x] Everything at or below `MAX_DATA_SEGMENT` used to ride the control
  session, so `--streams N` bought **zero** parallelism on the workload that is
  slowest — for congress, 100% of the corpus. Small files are now striped
  across the data sessions, balanced by **count rather than bytes**, because at
  this size the per-file cost dominates the payload.

Also wires the 4.26 apply pool into `run_data_sink`, which 4.26 had left out;
without it the striped files would have landed on a serialized receiver.

**Linux — it works, modestly.** freya, congress-100k, verified:

| | streams=1 | streams=2 | streams=4 | streams=8 |
|---|---:|---:|---:|---:|
| post-4.25 | 9.20 s | 7.77 s | **6.98 s** | 7.17 s |

Same-stream-count A/B at `--streams 4`, alternating: **7.59 s → 6.75 s, 1.12×**.
Best absolute is 6.75 s against 8.46 s single-stream — **1.25×** — and it
plateaus at 4.

The gain is smaller than the premise implied, and the reason is 4.26: a single
receiver already applies files across 8 threads, so extra connections add
receivers to a path that is no longer receiver-starved.

**Windows — streams remain harmful, and striping does not rescue them.**

| | streams=1 | streams=2 | streams=4 |
|---|---:|---:|---:|
| congress-100k | **56.62 s** | 63.64 s | 65.97 s |

Two streams cost 12% and four cost 17%. Whatever serializes NTFS file creation
is not relieved by more concurrent writers — the apply pool already gives each
receiver 8 threads, and adding receivers contends rather than parallelises.

**What this settles for 4.14.** The byte-share gate is wrong in both
directions. Small-file corpora *do* benefit from streams on Linux, so gating on
"share of bytes in files above `MAX_DATA_SEGMENT`" would wrongly refuse them.
But the platform cap dominates: on Windows the right stream count is 1
regardless of corpus. **Platform first, corpus second** — the opposite priority
to the one 4.14 currently specifies.

### 4.26 — Receiver-side parallel apply *(done 2026-08-30)*

- [x] The receive loop decoded **and applied** on one thread. Decoding must stay
  serial — it is an ordered stream — but publishing a file (write temp, verify,
  set metadata, rename) is independent per file, and was the serialized half.

**Design.** An `ApplyPool` of `min(cores, 8)` threads shares the `Sink` behind an
`Arc`; the decode thread does only the cheap, `&mut self` parts (batch-record
lookup, path-uniqueness validation) and hands the rest to the pool.
`XSYNC_APPLY_WORKERS` overrides the count for measurement.

**The ack contract is unchanged, which is what makes it safe.** A file is still
acknowledged only after it is durably renamed into place. Acks now leave out of
order, and that was already safe: the sender's `drain_acks` counts
acknowledgements and never matches them to ids.

**The deadlock this had to avoid, found by a hanging test.** The sender drains
to *zero* at every batch boundary. If the receiver blocks on the next read while
files are still in flight, it is holding acknowledgements the sender is waiting
for, and both sides stop. `active_files` empties exactly when a batch completes,
so the receiver drains the pool fully at that point and overlaps freely within
the batch. Acks are written buffered and flushed before the decode thread can
ever block on input.

**Measured — client held constant, server binary alternated, every run verified**

| server | reps (s) | median | files/s | gain |
|---|---|---:|---:|---|
| freya, before | 14.17, 12.30, 12.36 | 12.36 | 8,867 | — |
| freya, after | 8.69, 8.46, 8.33 | **8.46** | 12,961 | **1.46×** |
| Windows, before | 90.25, 90.34 | 90.30 | 1,215 | — |
| Windows, after | 55.75, 55.91 | **55.83** | 1,964 | **1.62×** |
| orion (Pi 5), before | 11.63, 12.79 | 12.21 | 8,977 | — |
| orion (Pi 5), after | 9.43, 9.02 | **9.23** | 11,875 | **1.32×** |
| WSL2 ext4, before | 18.69, 19.23 | 18.96 | 5,782 | — |
| WSL2 ext4, after | 15.54, 15.70 | **15.62** | 7,018 | **1.21×** |

**Windows gains more, as the story predicted** — it is the receiver-bound
platform, and unblocking the receiver is worth more there than on Linux.

**4.15 and 4.26 multiply, as claimed.** congress-100k to freya has gone
26.30 s → 12.80 s (4.15) → **8.46 s** (4.26): **3.11× end to end**.

**Followed up in 4.25**: the pool is now wired into `run_data_sink` as well,
which that story needed once small files began travelling the data path.

### 4.27 — io_uring *(investigated and declined 2026-08-30)*

- [x] Measured rather than implemented. **The receiver is syscall-heavy but not
  syscall-limited**, so batching syscalls cannot buy much.

**The receiver is genuinely kernel-bound in CPU terms.** Undistorted
(`bash time`, no tracer), publishing congress-100k on freya:

```
  7.492 s real   1.982 s user   12.842 s sys
```

System time is **6.5× user time** — this is a syscall-dominated workload, and
that is the case *for* io_uring. But 12.84 s of system time spread over 7.49 s
of wall clock is only ~1.7 cores of an available 32, across eight apply threads.
The workers are waiting, not saturating.

**The decisive test is worker scaling.** If syscall cost sat on the critical
path, adding appliers would keep helping:

| apply workers | 1 | 2 | 8 | 16 |
|---|---:|---:|---:|---:|
| congress-100k | 11.45 s | 9.23 s | **8.75 s** | 8.80 s |

Going 1 → 2 is worth 1.24×, 2 → 8 is worth **1.05×**, and 8 → 16 is worth
nothing. The pool has already captured essentially all the parallelism the
receiver has to give; the asymptote is ~8.7 s and we are on it. Halving the cost
of each syscall moves a component that is already off the critical path.

**Verdict: declined.** io_uring needs an `unsafe` exemption (the second in a
crate that has exactly one, documented), is Linux-only behind a runtime probe,
and targets a few percent of wall clock. 4.11's precedent applies — a permanent
`unsafe` exemption needs a measured win, and this one is not there.

> **A profiling trap worth recording.** `strace -f -c` on the receiver
> attributed **70.8% of syscall time to `futex`** and only ~24% to real file
> I/O, which pointed hard at lock contention in the 4.26 apply pool. Two global
> `Sink` mutexes were indeed taken per file, so the diagnosis looked sound.
> Removing them changed wall clock by **nothing** (8.85 s → 8.80 s, alternated).
>
> `strace` inflates per-syscall cost by roughly an order of magnitude, and it
> inflates *cheap, frequent* syscalls hardest — which is exactly what a
> contended futex looks like. **`strace -c` percentages are a distribution of
> traced syscalls, not an attribution of wall clock.** The `time` measurement
> above took two minutes and was worth more than the whole trace.

**Kept anyway, as a simplification.** `Sink::temporary_path` held a global
`Mutex<HashMap>` purely to memoise a BLAKE3 hash of a short relative path —
tens of nanoseconds of work behind a lock that eight threads contend for. That
cache cost more than it saved even if the wall clock could not see it. The map
is gone and `ensured_directories` is now an `RwLock`, since it is read-mostly.
**No measured gain**; less code, one fewer allocation per file.

### 4.28 — Parallelism topologies, surveyed *(measured 2026-08-30)*

- [x] All four sized on a Linux-to-Linux pair (freya → orion, congress-100k,
  baseline **7.53 s**) so no OS difference contaminates the answer. Two are
  dead, two are worth roughly 10% each. **The step changes are gone** — 4.15,
  4.25 and 4.26 took them.

**Where the time goes now.** Neither end is CPU-saturated on a 32-thread sender
and a 4-core receiver:

| | real | user | sys | cores avg |
|---|---:|---:|---:|---:|
| sender (freya) | 7.62 s | 4.12 s | 5.44 s | 1.25 |
| receiver (orion) | 7.49 s | 1.98 s | 12.84 s | 1.71 |

#### 1. Multiplexed logical streams over one connection — **declined**

There is nothing to reclaim. 4.7 already measured connection setup as flat from
1 to 16 streams, and on this pair extra streams are a *net loss*:
`--streams 4` costs **8.01 s** against **7.54 s** single-stream. Making stream
count free is worthless when the streams themselves do not pay.

#### 2. Scan/transfer overlap — **the largest remaining item, ~10%**

`--dry-run` isolates scan and planning with no transfer:

| | freya → orion | macOS → freya |
|---|---:|---:|
| scan + plan | **0.79 s** | **1.45 s** |
| full transfer | 7.53 s | 7.70 s |
| share, before the first byte moves | **10.5%** | **19%** |

All of it is dead time on the wire. Transfer start is gated on plan completeness
per kind, so this is recoverable in principle by streaming the classification
into the sender rather than completing it first. It is the single biggest
identified slice left.

#### 3. Parallel compression — **real but bounded, ~5–9%**

Compression is demonstrably *on* the sender's critical path — the wall clock
tracks its cost:

| `--compress-level` | 1 | 3 (default) | 9 |
|---|---:|---:|---:|
| congress-100k | 7.14 s | **7.54 s** | 15.23 s |

Level 9 doubles the transfer, which proves the path is compression-sensitive.
But at the default the headroom is small: dropping to level 1 buys only **5%**,
so perfectly parallelising level-3 compression is worth on the order of 5–9%,
not more.

**Compression itself must stay on.** It is a **1.66× win** on this corpus —
7.53 s against 12.52 s with `--no-compress`. The question was only ever whether
to parallelise it, never whether to keep it.

#### 4. Hash parallelism — **declined, and currently unavailable**

`blake3` is built with `prefer_intrinsics` but **not** the `rayon` feature, so
`update_rayon` is not compiled in. Enabling it would only affect inputs at or
above `MAX_DATA_SEGMENT`, and large-file transfers are transport-bound —
64.9 MB/s against an 86.5 MB/s `ssh` ceiling. A faster hash cannot move a
transfer that is waiting on the cipher.

**Still deliberately not pursued**: multi-process sharding and GPU hashing.

### 4.56 — Overlap planning with transfer *(medium priority)*

- [ ] Transfer start is gated on plan completeness per kind, so scanning and
  classification are dead time on the wire. Measured with `--dry-run`, which
  performs exactly that work and stops:

| | freya → orion | macOS → freya |
|---|---:|---:|
| scan + plan, no transfer | **0.79 s** | **1.45 s** |
| full transfer | 7.53 s | 7.70 s |
| share before the first byte | **10.5%** | **19%** |

**This is the largest identified slice left.** 4.28 sized the other three
candidates: multiplexed streams and hash parallelism are dead, and parallel
compression is worth 5–9%. After 4.15, 4.25 and 4.26 there is no step change
remaining, and two ~10% items are what is on the table.

**Why it is not free.** The sender cannot classify an entry until the
destination index says whether it exists and differs, and that index arrives
from the peer. What *can* overlap is the source scan and the local half of
planning against the destination scan's arrival, and then the transfer of
already-classified entries while later ones are still being decided.

`classify_stream` already exists and streams entries through a callback rather
than materialising a `Plan`, so the shape is present; the push path calls
`try_plan_with_fingerprint` and waits for a complete `Plan` instead.

**AC**

- The push path consumes classifications as they are produced and begins
  sending `New`/`Changed` files before the whole tree is classified.
- Ordering guarantees that currently hold are stated and preserved:
  directories are created before their children, and `--delete` still runs
  after a successful transfer, not interleaved with it.
- The dry-run share is re-measured afterwards on both pairs. Success is
  recovering a real part of the 10.5%/19%, not merely moving the work.
- Failure semantics unchanged: a scan error must still abort before anything
  is published, which is harder once publication starts early — this is the
  main risk and should be tested explicitly.
- Measured with the usual discipline: verified runs, warmup discarded,
  alternating arms, on the Linux pair first so no OS difference intrudes.

**Not to be confused with** the sender read-ahead in 4.15, which overlaps
reading with sending *inside* the transfer phase. This overlaps the phase
boundary itself.

### 4.57 — The rsync fallback cannot pull

- [x] `rsync.rs` now implements `sync_pull` alongside `sync_push`.
  `--transport rsync` and the `auto` fallback therefore work for both uploads
  and downloads when a peer has no `xs`.

**Why it matters more than it looks.** The rsync transport is what makes `xs`
usable against hosts the user does not administer: it speaks the rsync wire
protocol (v32) natively to the remote's own `rsync --server`, needing no local
`rsync` and no remote `xs`. That is the rung that turns "an optimisation for
machines you control" into "works nearly everywhere" — and today it only holds
weight in one direction.

Surfaced by the Kestrel review (`../sftp/backlog-xs.md`, XS-A2b): a file browser
downloads at least as often as it uploads, so a pull-less fallback drops half
its traffic straight to a per-file protocol.

**AC**
- `sync_pull` over the rsync wire at parity with `sync_push`: recursive trees,
  resumable where the protocol allows, the same `LocalEvent` stream, and the
  same transient/fatal error classification.
- `--transport auto` selects it for pulls on the same terms it already selects
  it for pushes, and the run summary names the transport actually used.
- The existing refusals stay honest rather than being silently widened:
  `--delete`, include rules, `--streams > 1` and `--paranoid` are refused on
  this transport today, and each must either work for pull or be refused with
  the same message it gets on push.
- Round-trip tested against a real GNU rsync peer, not only against ourselves —
  the push path already has that discipline in `rsync_wire` tests.

**Explicitly out of scope**: making the rsync transport reach feature parity
with the native one. It is a compatibility rung, not a second first-class
backend, and it should stay small enough to stay correct.

**Delivered:** `sync_pull` now drives GNU rsync protocol 32's sender-side
session without a local rsync executable. It reads and re-sorts the remote file
list using GNU's index ordering, requests whole-file literals, preserves the
v1 recursive file/directory/symlink subset, and completes the sender's
three-phase goodbye. Explicit rsync and `auto` fallback both select it for
remote-to-local transfers. Integration coverage drives a real GNU sender with
the local `rsync` hidden from `PATH`.

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

*The VM shuts itself down mid-benchmark, and `vmIdleTimeout=-1` does not stop
it.* WSL terminates the VM shortly after the last **`wsl.exe` client on Windows**
exits. An inbound sshd session into the distro does **not** count as a client, so
a benchmark driven entirely over port 2222 boots the VM, transfers for ~13 s, and
then dies with `Broken pipe` when the VM disappears underneath it. A keepalive
started over SSH does not help either: sshd's job object kills it when that
session ends. What works is holding a `wsl.exe` client open for the duration from
the *driving* machine:

```
ssh sanjee@192.168.1.120 "wsl --cd / -d Ubuntu -- sleep 3600" &
```

Every WSL measurement must pin the VM this way, and must verify results rather
than trust them — see the retraction in 4.48 for what this cost.

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

### 4.48 — The OS penalty on identical hardware *(measured 2026-08-30)*

- [x] Same Mac, same source, same binary, pushed to one machine (7900X, one
  NVMe, one link). **Every figure below is verified**: non-zero exit rejected
  and the landed file count checked against the corpus.

**Small files — congress-100k (109,615 files, 850 MB)**

| destination | userspace + filesystem | median | files/s |
|---|---|---:|---:|
| native Windows, as configured | Win32 + NTFS | 87.23 s | 1,257 |
| native Windows, Defender-excluded | Win32 + NTFS | 71.39 s | 1,536 |
| WSL2 | Linux + ext4 (VHDX, same NVMe) | 18.70 s | **5,862** |
| WSL2 `/mnt/c` | Linux + NTFS over the 9p bridge | ~2,900 s est. | ~32 |

**Linux is 4.66× faster than Windows on identical hardware** — same CPU, same
NVMe, same link, same source tree. With Defender excluded it is still **3.82×**.
This is the first version of `docs/OS.md`'s "the OS is worth ~6×" that holds the
hardware constant, and it lands somewhat below that figure.

**Large files — 3.94 GiB in 7 files (mean 576 MB)**

| destination | median | MB/s |
|---|---:|---:|
| native Windows, as configured | 62.65 s | 64.4 |
| native Windows, Defender-excluded | 65.98 s | 61.2 |
| WSL2 ext4 | 62.20 s | 64.9 |
| freya (different box, native Linux) | 66.79 s | 60.3 |

**The OS penalty disappears entirely on large files.** Everything lands within
7% — but **not because the link is saturated**, which is what this first said.
Measured on the same pair: a plain `dd | ssh 'cat >/dev/null'` stream moves
**86.5 MB/s** (2 GB verified), against xsync's **64.9 MB/s**. The link is 1 GbE
(a USB adapter, `1000baseT`), practical ceiling ~112 MB/s. So the convergence
sits at **58% of the link and 75% of a plain SSH pipe**: the common ceiling is
xsync's own large-file path plus SSH, not the network. That is ~25% of headroom
on large files that nothing platform-specific is protecting. So the honest headline is
**"the OS costs 4.66× on small files and nothing on large ones"** — a much more
precise and more useful claim than "the OS is worth ~6×", and one that matches
the mechanism: the penalty is per-file metadata cost, and large files amortise
it to nothing.

**`/mnt/c` remains guidance only.** 9p is a per-operation RPC, so ~32 files/s
measures the bridge, not NTFS. Never sync into `/mnt/c`.

**The NTFS-vs-Win32 decomposition stays dropped** — see the reasoning below; the
large-file row above already establishes that the cost is per-file, which is the
part that matters for engineering.

> **Two retractions, both mine.**
>
> This row briefly read **13,964 files/s and 11.1×**. Those runs never happened:
> the WSL VM was intermittently shut down, `xs` was failing in ~7.85 s, and
> `-q >/dev/null 2>&1` with no exit-code check turned every failure into a
> plausible number. 513 MB/s over a gigabit link is what finally exposed it.
>
> Worse, I explained the bogus figure with a fabricated mechanism — "the first
> large write expands the ext4 VHDX" — which sounded plausible and was invented
> to fit bad data. The verified re-run gives 18.70 s, within noise of the
> original 19.70 s. There was no warmup effect. **A mechanism proposed to
> explain a surprising number must be tested before it is written down.**

### 4.49 — Re-measure 4.15 idle *(closed 2026-08-30 — twice, and the second pass corrected the first)*

**Final answer: 4.15 is 2.05×–2.15×.** Idle host, wired link, alternating arms,
every run verified by exit status *and* landed file count:

| session | conditions | before | after | ratio |
|---|---|---:|---:|---|
| first | freya loaded (`xc` ~950% CPU) | 24.88 s | 13.36 s | 1.86× |
| second | freya idle | 24.79 s | 11.54 s | 2.15× |
| third | freya idle, wired, verified | 26.30 s | 12.80 s | **2.05×** |

**The premise this story was filed on was false**, and so was the conclusion I
first drew from disproving it.

The filing assumed contention had inflated the *before* arm. It had not — idle
and loaded `before` are indistinguishable. That left the 14.4 s recorded earlier
in the cycle for equivalent code unexplained, and I wrote it up as *"the same
commit is simply slower on this host today"* — i.e. as **host drift**, which
would cast doubt on every freya number ever taken.

That was the wrong conclusion. The same `before` code has now been measured in
three separate sessions at **24.88, 24.79 and 26.30 s** — a 6% spread across
load states, link states and rebuilds. **This host is stable.** The 14.4 s
figure is a lone 1.8× outlier that reproduces under *no* tested configuration:
not with matched pre-4.5 client and server (`bb5d6a06`: 24.98 / 24.46 s), not
idle, not wired, not verified. Load, ZFS fragmentation (`FRAG 12%`, `CAP 7%`,
unchanged), CPU governor and network path are all ruled out.

**It should therefore be treated as an erroneous measurement, not as evidence of
drift** — a contained problem rather than one that taints the record. It was
taken before verification was routine, in the same stretch that produced the
retracted WSL numbers in 4.48, and it verified only the final repetition's file
count.

**The durable lesson stands and is now sharper.** A single-arm number from hours
earlier was treated as a baseline and manufactured a false regression scare, then
a false drift conclusion on top of it. Only interleaved same-session A/Bs with
per-run verification have survived contact with reality this cycle. 4.21 must
make both the default.

### 4.61 — Pull re-read and re-hashed the whole file for every 8 MB chunk

- [x] **Fixed 2026-08-31.** The server's `LargeFileRange` handler called
  `source_reader.read(entry)`, which buffers *and BLAKE3-hashes the entire
  file*, then kept one 8 MB slice and threw the rest away. Serving a 500 MB
  file in 61 chunks read and hashed roughly **30 GB to move 500 MB**.

The cost per chunk therefore scaled with **file size**, not chunk size --
quadratic in the number of chunks. Measured directly from the `--progress-json`
event stream: **266 ms median per 8 MB chunk** (deciles 189→279, no
bimodality), where 8 MB of wire time is ~70 ms.

| arm | before | after |
|---|---:|---:|
| rsync pull | 109.9 MB/s | 110.5 MB/s |
| xsync pull | **29.7 MB/s** | **73.4 MB/s** |

**2.47x**, and the gap to rsync closes from 3.7x to 1.51x. All three files
verified identical by SHA-256 against the source after the change.

`read_range` already existed and reads only the requested bytes; the handler
simply never used it. It also drops the whole-file buffer, which matters for
4.23 on the 3 GB Pi.

**A second bug had to be fixed to use it.** `read_range` checks the file's
fingerprint before and after the read, and the entry it checks against was
reconstructed *from the wire* in the `LargeFilePrepare` handler with
`ctime: None, unix: None` -- values the protocol cannot carry. Such a
fingerprint can never equal a real `stat`, so every range read failed with
"source file changed during read". The old code masked this because `read`
retries and adopts a refreshed fingerprint on mismatch.

The server now derives the fingerprint from its **own** filesystem. That is the
correct behaviour independently of performance: accepting the peer's
description of a local file made the replacement check compare against a value
that was wrong by construction, so it could never have detected a genuine
mid-transfer replacement. `test_pull_matches_push_identically` caught this.

**Diagnosis note.** The first hypothesis was per-chunk `fsync` cost -- the pull
loop issues three durability barriers per chunk (`sink.sync_data`, the
journal's `sync_all`, and the parent-directory `sync_all`), and macOS
`F_FULLFSYNC` is expensive. Measured on this Mac, those three total **~21 ms**,
not the ~196 ms of overhead observed. The hypothesis was wrong and measuring it
was what pointed at the file-size scaling instead.

**Still open**: the pull loop remains lockstep -- request, segment, ack,
range-ack per chunk, with the local write and a journal checkpoint serialized
in between. That is the pull analogue of 4.60 and is the likely bulk of the
remaining 1.51x.

### 4.60 — The large-file path has no pipelining, and rsync is 1.8x faster *(PRIORITY)*

- [x] **Push path fixed 2026-08-31.** Pull is unfixed; see below.
- [ ] `run_client_push`'s large-file loop sends one 8 MB chunk and then blocks
  on two acknowledgements before reading the next. `max_pipelined_frames()` --
  the window every other send path uses -- is never consulted here. Transfer and
  receiver-side work never overlap.

```rust
for range in missing {                     // 8 MB chunks
    write LargeFileRange frame
    blake3::hash(chunk)
    write FileSegment
    let ack1 = decoder.read(&mut reader)?; // BLOCK
    let ack2 = decoder.read(&mut reader)?; // BLOCK
}
```

**Measured 2026-08-31, Mac -> mars, 1 GbE, 10 files / 4.32 GiB of already-
compressed `.cbz`.** Three arms interleaved in one session, every run verifying
exit status and landed file count:

| arm | runs (s) | median | throughput |
|---|---|---|---|
| rsync (over the same ssh) | 37.63 / 38.42 / 37.87 | 37.87 | **114.0 MB/s** |
| xsync -> ext4 | 65.68 / 71.57 / 68.62 | 68.62 | **62.9 MB/s** |
| xsync -> tmpfs | 49.88 / 49.17 / 49.29 | 49.29 | **87.6 MB/s** |

The gap decomposes, and both halves are the same root cause:

- **1.39x** -- the receiver's disk write, serialized into the transfer window
  (ext4 vs tmpfs, everything else held constant).
- **1.30x** -- residual after removing the disk: BLAKE3 verification and the two
  blocking acks per chunk (tmpfs vs rsync).

**Neither end is busy.** Sampled during the runs: rsync sender CPU median 15.1%,
receiver 4.8%; xsync sender **8.4%**, receiver 7.2%. xsync burns *half* the
sender CPU of rsync and takes 1.8x as long -- it is not losing a computation, it
is waiting. The operator noticed the same thing from across the room: rsync
spins the fans up and xsync does not.

**This corrects 4.50 and 4.17 on a point that matters.** Those recorded two
ceilings below the wire and attributed ~23% to SSH itself, from a
`dd | ssh cat` measurement of 86.5 MB/s. On this pair `dd | ssh cat` gives
**114.8 MB/s**, and rsync -- which runs over the very same `ssh
mars.local rsync --server ...` -- reaches 114.0. **There is no SSH tax here.**
The 86.5 figure was a property of the Mac<->Windows pair, not of SSH, and the
whole remaining gap belongs to xsync. Wherever those stories say the large-file
ceiling is shared between SSH and xsync, it is xsync's alone.

**Why this is the priority.** It is the ceiling the 10 GbE link exists to
probe. A serialized round trip per 8 MB does not get cheaper when the wire gets
faster: the wire time per chunk shrinks 10x while the receiver's hash and write
do not, so the *share* lost to the stall grows. xsync would gain far less than
rsync from the X540, and could plausibly not move at all.

**Not the explanation, checked:** compression (`--no-compress` is no better on
already-compressed input), thermal or disk drift (rsync held 114 MB/s
interleaved between every xsync run), and the link (rsync saturates it).
`--streams` is a *partial, already-known* mitigation -- 4.14 measured ~1.2x on
Manga peaking at 4 -- which would reach ~96 MB/s at best and still trail rsync.
It hides the stall behind concurrent connections rather than removing it, and
costs an SSH connection each.

**AC**

- The large-file loop pipelines chunk sends against the same window the batch
  path uses, draining acks at a low-water mark rather than after every chunk.
- The resume journal's durability contract is preserved: a chunk still counts
  as verified only when its ack is seen, so an interrupted transfer resumes
  correctly. Pipelining changes *when* acks are collected, never whether.
- Re-measured against rsync on the same interleaved protocol. The target is the
  link, not a percentage.
- Re-run on 10 GbE once the X540 lands, since that is where the remaining
  per-chunk cost becomes visible.

**Delivered (push).** The loop now keeps a bounded number of chunks in flight
and drains acknowledgements at a low-water mark. Depth is a *byte* budget
expressed in chunks -- `DEFAULT_UNACKNOWLEDGED_WINDOW / LARGE_FILE_CHUNK` = 4,
32 MB -- deliberately not the frame window the batch path uses, because these
frames are 8 MB each rather than ack-sized. Overridable as
`XSYNC_LARGE_CHUNKS_IN_FLIGHT`; **`1` reproduces the old lockstep exactly** and
is the control arm below.

The durability contract is unchanged. All chunk acks are drained to zero before
`LargeFileFinish` is sent. That is required for two reasons, and the second is a
silent-corruption hazard rather than a performance one: the receiver commits on
Finish, and a straggling chunk ack is itself a `Message::Ack`, so it would have
satisfied the Finish ack check without complaint.

**Measured, tmpfs destination to remove receiver-disk variance, rsync
interleaved as a drift anchor.** Two corpora, agreeing within 2%:

| arm | 4.32 GiB | 0.98 GiB |
|---|---:|---:|
| rsync | 114.0 MB/s | 111.3 MB/s |
| xsync, `=1` (old lockstep) | 86.9 MB/s | 87.0 MB/s |
| xsync, `=4` (new default) | **102.2 MB/s** | **102.8 MB/s** |
| xsync, `=16` | 101.9 MB/s | -- |

- **1.18x on tmpfs**, and **1.32x to ext4** (62.1 -> 81.9 MB/s), where the
  receiver's disk write is also serialized.
- **Depth 4 and 16 are indistinguishable.** 32 MB in flight already saturates,
  so the negotiated window is the right default and needs no tuning. A deeper
  window is not where the remaining gap lives.
- Remaining gap to rsync: **1.11x on tmpfs**, 1.37x to ext4.

**A methodology note worth keeping.** A depth sweep run *sequentially*, without
an anchor, produced non-monotonic nonsense -- depth 4 measured 68.97 s minutes
after the same configuration measured 52.70 s. Repeated multi-GB writes leave
ext4 writeback in varying states. Anchoring on rsync and destroying the disk
variable with tmpfs turned the same experiment into tight, reproducible
numbers. Sequential sweeps of this workload cannot be trusted.

**Still open.**

1. **`run_client_pull` has the same lockstep loop** (a second copy of this code
   at the pull site). Downloads did not get this fix.
2. **The remaining 1.11x on tmpfs.** Candidates, unmeasured: the whole file is
   read into memory before any chunk is sent (`source_reader.read` returns
   `stable.bytes` for the entire file), so disk read and network never overlap
   *between* files either; and BLAKE3 is computed inline per chunk in the send
   loop.
3. **The ext4 gap is larger than the tmpfs gap**, so receiver-side write
   serialization remains worth attacking separately.

**Watch for** the 8 MB chunk buffer: the loop does `stable.bytes[start..end]
.to_vec()` per chunk, and appears to hold the whole file in memory. Pipelining N
chunks must not become N more full-size copies, which is a real constraint on
the 3 GB Pi (4.23).

### 4.58 — The rsync-fallback tests are flaky

- [ ] `test_auto_falls_back_to_rsync_for_remote_source_when_xsync_is_missing`
  fails roughly **40% of the time** under the full parallel suite and passes
  every time when run alone. Measured at `3e810898` with no local changes: two
  of five runs failed, one of them with two failures.

The observed error is `remote rsync exited unsuccessfully (status 10): rsync:
[sender] write error: Broken pipe (32)` — a teardown race, not a wrong result.

**This probably also explains the musl CI failure.**
`test_auto_does_not_fallback_on_host_key_or_native_protocol_failure` was
reported as musl-specific, counting two backend invocations where the test
demands one. It is in the same family — same fake-rsh harness, same
marker-file invocation counting — and a flaky sibling is a much cheaper
explanation than a real musl divergence in fallback safety. Confirm which
before treating it as a platform bug.

**Why it matters beyond the noise.** These tests assert that `auto` does *not*
silently switch transports when the native protocol misbehaves. A test that
fails 40% of the time for unrelated reasons cannot enforce that property,
because a real regression is indistinguishable from the usual noise.

**AC**
- The failure is diagnosed, not retried away. `#[serial]` is acceptable only
  once the actual race is understood and named.
- 50 consecutive full-suite runs with no failure.
- The musl question is resolved either way and recorded.

### 4.59 — A sender window smaller than the receiver's apply pool deadlocks

- [x] Fixed in the same change that exposed it, but recorded because the
  invariant is cross-process and cannot be checked at run time.

The receiver acknowledges a file only on durable rename, so it can hold up to
`capacity` jobs un-acked, where `capacity = apply_workers * 8`. If the sender's
pipelining window is no larger, the sender blocks waiting for acks the receiver
is waiting for more work to produce. Both stop. Measured with capacity 64: a
window of 32 lands 31 files and hangs indefinitely; 64 completes.

**This was reachable before any of the new tuning knobs existed.**
`XSYNC_APPLY_WORKERS` had no upper bound, so `=1000` built a pool of 8,000
un-acked jobs and deadlocked against the stock 2048-frame window.

Now held structurally in `tuning.rs`: a floor of 512 on the window, a ceiling
of 32 on workers, and a compile-time assertion the two cannot overlap.

**Still open, and worth deciding before the daemon phase.** The guard is a
static bound, not a check. Two ends can still be built from different commits
with different constants, which is exactly the mixed-version case R1.4 in
`backlog-release.md` says a rollout guarantees. If the window and the pool
capacity were exchanged during negotiation, this would be a checkable
invariant rather than a maintained coincidence.

### 4.50 — A 2.5 GbE point-to-point link, and what it is for

- [ ] Every cross-host number in this project was taken over a 1 GbE link
  reached through a **USB dongle on the Mac**, and nothing currently gets near
  even that ceiling.

| transport | throughput | of 1 GbE |
|---|---:|---:|
| 1 GbE practical ceiling | ~112 MB/s | 100% |
| `dd \| ssh 'cat >/dev/null'` | 86.5 MB/s | 77% |
| Wi-Fi 6 (5 GHz, 80 MHz), same test | 72.3 MB/s | 65% |
| xsync, 3.94 GiB in 7 large files | 64.9 MB/s | 58% |

**The point is not bandwidth.** It is that two ceilings already sit below the
wire — SSH costs ~23%, xsync a further ~25% — and they cannot be told apart
while the link is close enough to matter. More headroom turns "everything
converges at 64.9 MB/s" into a decomposable measurement.

**Why 2.5 and not 10.** The Windows box already has a *Realtek PCIe 2.5GbE*
controller, auto-negotiated down to 1 Gbps only because the other end is
gigabit. The missing piece is a USB-C 2.5GbE adapter on the Mac and a direct
cable — **no switch, static addresses, ~280 MB/s**, roughly 3.3× the current
SSH ceiling. 10 GbE needs a PCIe card *and* a Thunderbolt adapter, and while
`ssh` itself tops out at 86.5 MB/s it would largely measure OpenSSH.

**Thunderbolt peer-to-peer was investigated and ruled out.** The Mac has
Thunderbolt Bridge (`bridge0`, `en1`–`en3`, 40 Gb/s buses), but the Gigabyte
X870 exposes only a Microsoft generic USB4 host router with no `Net`-class
device. IP-over-Thunderbolt on Windows is an Intel-driver feature; a USB-C
cable between these two enumerates without producing an IP link.

**Latency may matter more than bandwidth here.** The in-session SSH round trip
measured in 4.15 was **5.3 ms**, absurd for a LAN, and it is why widening the
pipeline window was worth 1.26×. The USB dongle is the prime suspect. Small-file
sync — where every interesting result in this project lives — is latency-bound,
not bandwidth-bound, so the adapter change may pay off there first.

**AC**

- Direct 2.5 GbE link with static addresses, plus `dd | ssh cat` and raw-TCP
  baselines on it, so 4.17's transport table has a second link to compare.
- The 5.3 ms round trip is re-measured. If it drops materially, the pipeline
  window knee (2048, chosen at 5.3 ms) is re-derived, since it was tuned to a
  latency that may be an artifact of the dongle.
- congress-100k and the large-file corpus re-run on the faster link. The
  question is which ceilings move and which do not: a fixed per-operation cost
  will not scale with bandwidth, and that is how SSH crypto and xsync's chunked
  path get told apart.
- The 1 GbE figures are kept, not replaced. They are what most users have.

## Phase 12 — Native authenticated transport and daemon

SSH remains the shipping carrier until this phase clears both its security and
measurement gates. The goal is not to replace one mature security protocol with
home-grown crypto. The candidates are **QUIC (TLS 1.3 over UDP)** and **TLS 1.3
over TCP**, using maintained implementations and carrying the existing native
sync protocol. Pick one; do not ship and maintain both unless the measurements
show distinct regimes that justify two stacks.

This phase depends on 4.17 for the raw-TCP/SSH decomposition and benefits from
4.50's 2.5 GbE link. D6 in `DEPLOYMENT.md` owns service packaging; these stories
own the network protocol, trust model, and transfer integration.

### 4.51 — Choose QUIC or TLS-over-TCP with a transport spike

- [ ] **R.** Put the same bounded request/response and bulk byte streams over
  one QUIC connection and one TLS-over-TCP connection. This is a carrier test,
  not permission to fork the sync protocol or invent a new framing format.

**Experiment**

- Reuse the v2 handshake and representative encoded frames through a narrow
  transport adapter. Include one control flow plus 1, 4, 8 and 16 concurrent
  logical data streams over a **single authenticated connection**.
- Measure connection setup, first job, 100 repeated tiny jobs, congress-100k,
  and the large-file corpus on the 1 GbE and 2.5 GbE paths. Record wall time,
  throughput, CPU, memory, wire bytes and connection/process count beside the
  SSH and raw-TCP baselines from 4.17/4.50.
- Sweep 0/20/80/150 ms added RTT and controlled loss/reordering using 4.42.
  QUIC's UDP path must also be tried through a network where UDP is blocked;
  that failure is a product condition, not a lab anomaly.
- Build and run on macOS, Linux and Windows. Dependency review includes
  maintenance health, platform backends, binary cost, `unsafe` surface and the
  workspace's Rust-version floor. No custom TLS, certificate validation or
  congestion control.

**Decision gate / AC**

- One short decision record selects QUIC, TLS-over-TCP, or **neither**, with raw
  results and a stated reason. A second implementation survives only if it wins
  a measured regime the selected one cannot serve.
- The chosen carrier supports cancellation, backpressure and independent
  logical streams without head-of-line blocking between a slow file and the
  control path. If TLS-over-TCP wins, the record explains why connection-level
  head-of-line blocking is acceptable.
- The spike is throwaway unless it clears the identity design in 4.52. A fast
  unauthenticated socket is not progress toward the daemon.

### 4.52 — Peer identity, pairing and root authorization

- [ ] Define who a daemon trusts and what an authenticated peer may touch before
  opening a non-loopback listener. Encryption without authorization would turn
  `xs --server <root>` into a network-exposed filesystem API.

**AC**

- Mutual authentication is mandatory. The design specifies key generation,
  storage, peer naming, rotation, revocation and recovery on macOS, Linux and
  Windows. Private keys never live in the ordinary job config or logs.
- First pairing requires an explicit local act or an already-authenticated SSH
  bootstrap. There is no silent trust-on-first-use, shared default secret,
  anonymous write mode or `--insecure` path that can become a permanent setup.
- Authorization is allow-list based: a peer receives named roots and explicit
  read, write, delete and browse capabilities. Paths are resolved beneath the
  authorized root with the same collision, symlink and containment rules as the
  SSH server; a client cannot nominate an arbitrary absolute server path.
- Protocol negotiation is authenticated and fail-closed. Tests cover the wrong
  peer, expired/revoked credentials, replayed pairing material, key replacement,
  unauthorized roots/operations and allocation-amplification attempts before
  filesystem mutation.
- The threat model and credential locations are documented, including which
  local users can administer peers and read daemon state. V3.30/D8.2 consume
  this rather than maintaining a contradictory daemon security story.

### 4.53 — One daemon connection, many isolated sync sessions

- [ ] Turn the selected carrier into a long-lived, unprivileged daemon endpoint
  while keeping the existing one-shot `--server` state machine usable over SSH.

**AC**

- A connection has one authenticated peer identity and can carry repeated sync
  jobs plus control and data streams. Every job gets a fresh root authorization,
  options, cancellation scope, accounting record and cleanup boundary; state
  from one job cannot bleed into the next.
- `--streams N` means N logical data streams on one native connection, not N
  handshakes or N daemon processes. Control traffic remains responsive under a
  saturated bulk transfer, and per-job and per-peer limits bound streams,
  outstanding bytes, queued work, open files and idle time.
- Push, pull, browse, resume, progress and structured failures preserve their
  current application-protocol semantics. Carrier shutdown maps to stable
  transport errors rather than EOF guesses, and a cancelled or disconnected
  job leaves no published partial file or abandoned lock.
- The daemon runs as the logged-in user by default and never requires root to
  serve per-user data. Its foreground mode is fully testable without a service
  manager; systemd, launchd and Windows service/tray installation remain D6.1–D6.3.
- Graceful shutdown drains or cancels jobs within a bounded deadline. Crash and
  restart tests prove journal/resume recovery and credential state are not
  corrupted.

### 4.54 — Make carrier selection explicit and fallback safe

- [ ] Integrate the daemon without overloading today's `--transport` meaning.
  That flag currently chooses the native xsync protocol versus the rsync wire
  fallback; SSH versus the new daemon is a separate **carrier** decision.

**AC**

- CLI/config syntax distinguishes application protocol from carrier, has an
  explicit host/port form including IPv6, and defines precedence without
  breaking existing `host:path` SSH commands. Native-daemon use is opt-in until
  4.55 passes.
- Auto-selection may fall back from an unreachable/blocked daemon to SSH only
  before a job starts. It never falls back on an identity, authorization,
  downgrade, protocol-corruption or mid-transfer failure, and it never repeats
  a mutating job on another carrier by guessing that the first did nothing.
- No unauthenticated LAN discovery is required. If discovery is later added,
  advertisements are hints only and the pinned peer identity remains the
  authority.
- Human output and JSON report the application protocol, carrier, endpoint,
  authenticated peer, negotiated wire version, reuse versus new connection and
  fallback reason. They do not expose keys, tokens or full certificate bodies.
- SSH remains available as an explicit carrier and continues to pass its
  existing integration suite. A daemon-only peer produces an actionable error,
  not an attempted remote shell command.

### 4.55 — Native transport conformance, failure and performance gate

- [ ] Treat the native daemon as a new security boundary and delivery path, not
  as complete when a happy-path copy succeeds.

**AC**

- The same oracle-backed corpus matrix runs over pipe, SSH and the selected
  native carrier for push, pull, browse, resume, filters, metadata, sparse files,
  deletion and every supported stream count. Landed trees and structured final
  reports are equivalent apart from declared carrier fields.
- Deterministic failure tests inject handshake timeout, malformed frames,
  certificate/key rejection, stream reset, connection loss, daemon kill,
  cancellation, ENOSPC and restart during staging/publish. Each case has a
  bounded exit, stable error class and proven cleanup/recovery result.
- Protocol fuzzing runs both before and after authentication, with allocation,
  frame, stream and idle limits asserted. A security review covers dependency
  defaults, downgrade resistance, replay/0-RTT policy and denial-of-service
  exposure; mutating requests are never accepted as replayable early data.
- Interleaved, verified A/Bs against SSH run on both retained links and the RTT
  regimes from 4.51. To become the default carrier, the daemon must make
  repeated tiny-job latency at least **2× better**, improve large-file
  throughput or CPU cost by at least **20%** where SSH is the measured ceiling,
  and regress no retained small-file cell by more than **5%**.
- If the performance gate misses, the secure explicit carrier may remain for
  daemon/index functionality, but SSH stays the default and the result is
  recorded as such. If the security or failure gate misses, it does not ship.

## Phase 4 — Carried forward

Not ordered against each other; pulled from `backlogv3.md` because they are still
the highest-value open items once the above is done.

- **V3.20 — worker count should follow the storage, not the core count.** Four
  platforms of data will make this decidable. A runtime probe is the only option
  that fits all measured hosts.
- **V3.11 — make `--delete` survivable.** *Partly done 2026-08-30.* The
  catastrophic case is closed: a run that would remove **at least half of a
  destination of 100 or more entries** is now refused before the first removal,
  on every route — local, single-stream push, multi-stream push and pull.
  `--max-delete N` caps the set and doubles as the explicit authorisation, so
  one flag both restrains and permits. Verified: 500 local files and 400 remote
  files survived an unauthorised `--delete` (exit 1), and proceeded with
  `--max-delete`. Five planner tests cover the accident, explicit
  authorisation, an over-limit refusal, the small-destination floor, and a
  proportionate 20% prune that must *not* be blocked.
  **Still open from the v3 AC**: a `--backup`/trash mode, interactive
  confirmation, and delete summaries in the JSON output.
- **Same-filesystem fast path** (v3, unstarted): where both ends are the same
  filesystem, per-file work can be bypassed.
- **Verify-only mode** (v3): answer "are these two trees identical?" without
  writing.
- **One source to N destinations** (v3): read and hash once.
- **`protocol.md` framing.** It describes the wire as frozen. Either drop that or
  state that the freeze is a design goal, not a compatibility obligation —
  otherwise the next person routes around a constraint that is not there.
