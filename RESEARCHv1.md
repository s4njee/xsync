# xsync research v1

Research directions derived from the current [`README.md`](README.md),
[`BENCHMARKv2.md`](BENCHMARKv2.md), [`BENCHMARKSv3.md`](BENCHMARKSv3.md), and the
checked-in tuning decisions. This is a measurement agenda, not an implementation
backlog. An idea belongs here when an experiment can cheaply decide whether a larger
piece of engineering is justified.

The immediate constraint is important: the best current large-file paths are already
at, or close to, the practical ceiling of the available gigabit links. Network work
that can only raise bulk throughput should wait for the 10 GbE hardware. Work that
reduces latency, overlaps a slow receiver, improves local transfers, or changes the
amount of work remains measurable now.

## Evidence to carry forward

These results define the starting point. New work should not be justified using an
older, superseded number.

- On the switched gigabit path, large-file push reaches about 112 MB/s and pull about
  108 MB/s. On Linux receivers, xsync is at parity with rsync when comparisons include
  durability. More bulk-wire tuning cannot be resolved on this link.
- The mesh path is only about 792 Mbit/s; a ceiling measured through freya or orion may
  be the topology rather than either tool. See
  [`docs/network-topology.md`](docs/network-topology.md).
- The canonical physically connected network routes are Mac↔mars and Mac↔WSL2,
  both on the router1 switched segment. Mac↔freya crosses the mesh; orion is movable
  between router1 and router3 and must be labeled with its placement for every run.
- The remaining network exception is a slow destination. On orion, large-file push is
  about 83 MB/s to durability with one stream versus rsync at about 100 MB/s. The
  receive loop serializes network receive and disk write; eight streams hide enough of
  that stall to approach parity, but at the cost of eight SSH sessions and high
  variance.
- Network small-file performance is no longer the old headline loss. On
  `congress-100k`, xsync is 1.03x ahead from freya to orion and 1.13x behind when the Pi
  is the sender. The receiver pays the per-file cost; the sender and link pay the
  per-byte cost. **The "2.5x ahead from macOS to freya" figure is disputed and must not
  be carried forward until R0a cell 8 resolves it:** it rests on macOS→freya rsync taking
  20.24 s, but on 2026-08-31 macOS→mars rsync ran the same corpus in 7.76 s, and the mesh
  explains perhaps 13% of a 160% difference. Measured on the switched route that day,
  xsync led rsync by 1.08x, not 2.5x.
- Local transfers still have a large research surface. On Linux cross-NVMe,
  `congress-100k` measured 1.76x ahead of rsync, but a single-threaded tar pipe matched
  xsync. Cold `congress-1m` runs showed that worker scaling and the bottleneck change
  completely with cache state, filesystem, and device.
- Local APFS directory cloning is a categorical win: native `clonefile(2)` moved the
  `congress-100k` fresh-copy result from 28.31 s to 4.13 s, 5.82x ahead of rsync. This
  is more promising than making individual byte copies marginally faster.
- Windows remains the clearest per-file gap. Historical local results put xsync 1.74x
  behind robocopy on same-volume `congress-100k` and 2.97x behind cross-device. Network
  results on identical hardware show the Windows penalty shrinking monotonically as
  mean file size increases and disappearing on large files.
- The first persistent-index prototype did not meet its gate: 1.79M entries used about
  1.09 GB RSS, index build took 12.96 s, and planning took 2.70 s. The idea remains
  valuable, but that representation should not be integrated.
- Hashing is off the wall-clock critical path on the measured local workloads. Removing
  BLAKE3 entirely did not change wall time. `io_uring`, deeper large-file windows, SSH
  cipher selection, raw sockets, and directory-affine dispatch have also failed their
  current decision gates or have prizes too small to justify them.

One documentation caveat matters before any new comparison: the README performance
section still includes the old 0.515x local APFS result, while later tuning documents
contain native-clone and ext4 results that reverse it. The first research task is to
establish a current canonical matrix rather than infer the state of the current binary
from measurements of different revisions.

## Priority map

| Priority | Research question | Can run before 10 GbE? | Main payoff |
|---:|---|---|---|
| P0 | R0. What does the current binary actually spend time on? | Yes | Makes every later result comparable |
| P0 | R1. Can a receiver overlap large-file writes with network receive? | Yes, but see the deadlock | Closes the slow-destination gap |
| P0 | R2. Can local concurrency adapt to the OS, filesystem, device, and cache state? | Yes | Avoids known 10-20% losses and bad static defaults |
| P1 | R3. Which local copy primitive is fastest when integrity verification is mandatory? | Yes | Raises large local-copy throughput |
| P1 | R4. Which Windows operations account for the gap to robocopy? | Yes | Attacks the largest current per-file loss |
| P1 | R5. Can a compact persistent index make planning proportional to changes? | Yes | Changes re-sync from O(tree) to O(changes) |
| P1 | R6. Can sparse files be copied locally without materializing holes? | Yes | Turns an impossible 3.7 TB case into a ~130 GB copy |
| P2 | R7. When does clone-and-swap beat incremental mutation on APFS? | Yes | Extends the strongest local fast path |
| P2 | R8. Is there enough local cross-file reuse to justify a chunk index? | Yes | Avoids reads and writes, not merely network bytes |
| P2 | R9. Can scan, classification, and transfer safely overlap? | Yes | Recovers 10-19% startup dead time on measured routes |
| P2 | R10. Can corpus shape predict strategy better than corpus names? | Yes | Makes heuristics portable to unseen data |
| Deferred | R11. What becomes limiting at 2.5/10 GbE? | No, not decisively | Selects the next network architecture |

## R0 — Refresh the canonical baseline and phase accounting

**Hypothesis.** Several apparent open performance problems are stale documentation or
cross-revision comparisons. A current, phase-attributed matrix will change the priority
of at least one item below.

### R0a — The minimal trust anchor

**Everything else waits on R0, so R0 must be small enough to finish in one sitting.**
The full matrix below is a background fill; this subset is the gate. It is chosen to
answer exactly three questions: *is the current binary where we think it is*, *does the
receiver's disk explain the slow-destination gap*, and *which carried-forward numbers are
wrong*.

Rules for the anchor: one stamped release binary at the same commit on every endpoint
(verify with `xs --version` on both ends, not by assumption); rsync as the paired arm in
every cell; arms interleaved, never run in sequence; warmup discarded; three reps;
per-run exit status **and** landed file count verified; `--route-label` on every network
cell. Report wall **and** time-to-durable, plus receiver dirty bytes at exit — a
transfer-time-only comparison is not admissible after 4.64.

Corpora: `congress-100k` (109,615 files, 850 MB — metadata-bound) and a bounded ~1 GiB
incompressible large-file set (byte-bound). **cb7 is deliberately excluded from the
anchor**: at 59 GB it turns a one-hour gate into a multi-hour one, and nothing in R1 or
R2 depends on it. Initial copy only; no-op and churn workloads belong to R5 and R9.

| # | Cell | Corpus | Answers |
|---:|---|---|---|
| 1 | Mac → mars, ext4 | congress-100k | current small-file position on the switched route |
| 2 | Mac → mars, ext4 | large | current large-file position; expected at the link |
| 3 | Mac → mars, tmpfs | large | isolates receiver disk on a *fast* disk (control for 5-6) |
| 4 | Mac → orion, ext4 | congress-100k | small files where the receiver is slow |
| 5 | Mac → orion, ext4 | large | the R1 gap, restated on the current binary |
| 6 | Mac → orion, tmpfs | large | **the R1 hypothesis test**: does removing the disk close it? |
| 7 | mars → Mac, pull | large | pull is separate code and has its own history |
| 8 | Mac → freya, ext4 | congress-100k | settles the disputed 2.5x (label as mesh) |
| 9 | macOS APFS, same volume | congress-100k | settles the README's 0.515x local claim |
| 10 | mars ext4, cross-device | congress-100k | local Linux anchor for R2 |

Ten cells, two arms, four runs each. At roughly 8-12 s per run for these corpora that is
well under an hour of machine time, excluding orion's slower cells.

**Anchor gate.** Publish the ten-cell table and mark every contradicting row elsewhere as
historical. Two specific claims must be resolved by it, because both are currently
load-bearing and both are suspect:

- **"xsync is 2.5x ahead of rsync from macOS"** — **resolved: it is 1.24x.** Cell 8
  measured `rsync -a` at 10.94 s against xsync's 8.87 s on the same route. xsync barely
  moved (8.00 → 8.87 s); rsync halved (20.24 → 10.94 s), so the original baseline was
  erroneous. The gloss that macOS rsync is inherently slow is withdrawn — it runs this
  corpus in 6.86 s to mars.
- **The README's 0.515x local APFS result** — **resolved, and it was a corpus mismatch,
  not a stale number.** Every 0.515/0.534 figure was `congress-10k`; cell 9 measured
  `congress-100k` at **4.34x in xsync's favour** (5.79 s against 25.72 s). The comparison
  inverts with scale because xsync's local cost is nearly flat in file count (5.29 s at
  10k, 5.79 s at 100k) while rsync's is linear (2.82 s → 25.72 s). **A current
  `congress-10k` local run is now the missing measurement**, since the small-end figures
  predate the clone work.

**Deliberately not in the anchor:** cb7, Windows and WSL2 (see the scheduling conflict
under R4), no-op and churn workloads, and full phase attribution. The anchor establishes
*position*; the full matrix below establishes *where the time goes*.

### R0b — The full matrix

**Experiment.** Extend the anchor to the complete grid:

- local APFS same-volume and cross-volume;
- local Linux ext4 cross-device;
- local Windows same-volume and cross-device;
- macOS to fast Linux, macOS to orion, and the reverse direction;
- `congress-100k`, cb7, and a bounded 4-6 GiB incompressible large-file set;
- initial copy, no-op, and 1% content churn.

Report scan, destination index, plan, preflight, read/hash/compress, transfer/apply,
metadata, flush, and verification time. Record wall, user CPU, system CPU, peak RSS,
effective worker/stream counts, cache residency, destination dirty bytes at exit, and
time-to-durable. Phase time must account for at least 95% of wall time or name the
unaccounted interval.

**Decision gate.** Publish one current table and mark older contradictory rows as
historical. No optimization is promoted to P0 solely from a superseded result.

The release harness now consumes core-timestamped `phase-boundary` events, records an explicit
destination flush barrier (`time_to_durable_seconds`), best-effort dirty/writeback bytes and
`mincore` cache residency on local destinations, and captures endpoint metrics emitted by server
processes. Pass `--route-label` for every network cell (for example
`mbp-to-mars-switch`, `mbp-to-freya-mesh-router2`, or `mbp-to-orion-router1`) so mesh placement
cannot be mistaken for the physical router1 control route. Windows/WSL2 cells remain deferred
until mars is rebooted.

**Why first.** It is cheaper than another tuning patch, and the project has already
retracted multiple plausible conclusions caused by stale binaries, failed commands,
warmup, topology changes, or writeback differences.

## R1 — Receiver-side writer pipeline for large files

**Hypothesis.** A bounded writer pool can overlap receipt and verification of chunk N+1
with writing chunk N, achieving the benefit of eight streams without eight connections.

### This was attempted on 2026-08-31 and it deadlocked

Read this before designing the experiment. A `ChunkPool` modelled directly on the
existing `ApplyPool` -- bounded queue, worker threads writing off the decode thread,
acknowledgement and journal checkpoint both moved to write completion -- hung the test
suite and was reverted. See 4.66 in `backlogv4.md`.

**The cause is a flow-control coupling, not a bug in the pool.** The receiver blocks
reading the next segment. The sender blocks once its window of unacknowledged **chunks**
is full. If the receiver holds more outstanding writes than that window permits, it emits
no acknowledgement, the sender sends nothing further, and both sides wait forever.

`ApplyPool` is immune only by accident of scale: the small-file window is
`MAX_PIPELINED_FRAMES` (2048) against a pool capacity of 64. **The large-file window is
four chunks** (`DEFAULT_UNACKNOWLEDGED_WINDOW / LARGE_FILE_CHUNK`), so a pool of any
useful depth already exceeds it.

Two corrections to the original plan follow:

- **The unacknowledged *byte* window is not the memory bound that matters.** 32 MB is
  the memory ceiling; the four-chunk *count* is the liveness ceiling, and it is much
  tighter. Sizing the pool against the byte window is what produced the deadlock.
- **Capping outstanding writes at one was not sufficient.** With that cap,
  `test_durable_resume_skips_verified_ranges` still hung. The interaction between a
  partial chunk list on resume, the per-chunk `LargeFileRange` acknowledgement, and a
  deferred `FileSegment` acknowledgement was not run to ground and is the first thing to
  understand.

**The structural obstacle.** The sender's chunk window is a sender-side value
(`XSYNC_LARGE_CHUNKS_IN_FLIGHT`) that is **never negotiated**, so the receiver has no way
to learn how far it may safely run ahead. Any working design must therefore do one of:

1. exchange the window during the handshake, so the receiver can size itself under it;
2. stop using acknowledgement as the flow-control signal for chunk writes, e.g. ack on
   receipt-and-verify and carry durability separately; or
3. give the receiver a way to emit acknowledgements while blocked on input, which means
   restructuring the receive loop rather than adding a pool beside it.

**This makes R1 a protocol story, not a bounded prototype**, and its P0 ranking should be
read with that cost in mind. Option 2 is the smallest change but weakens what an
acknowledgement means, which the resume journal currently depends on.

### Experiment

Prototype a single-session pipeline with three bounded stages:

1. receive and decode;
2. verify the sender's range hash;
3. write, flush at the configured checkpoint boundary, update the journal, and ack.

Size stage 3 against the **negotiated chunk window**, not the byte window, and prove
liveness before measuring throughput: run with the sender window at its minimum, and with
a resume that leaves a partial chunk list, before trusting any timing. Preserve the
invariant that a range is journalled only after the bytes are durable, and do not allow
`LargeFileFinish` to publish until all writes and acknowledgements have drained --
`commit_temp` renames without syncing and the coverage check merges the on-disk journal,
so an outstanding write would both publish a hole and under-report coverage.

Run `streams=1` on orion ext4 and tmpfs, then use mars as the fast-disk negative control.
Also run a local two-device analogue to learn whether the same staged reader/writer shape
helps local copies.

**What success looks like is already measured.** R0a cell 6 gives the target directly:
xsync to orion tmpfs reached **108.3 MB/s** against ext4's 85.2 and rsync's 100.0 to
durability. The disk is worth 2.47 s on a 0.98 GiB transfer, and rsync is entirely
insensitive to the destination filesystem (8.77 s ext4 versus 8.73 s tmpfs). So the goal
is not "match rsync" -- it is to reach the tmpfs figure on ext4, which is *ahead* of
rsync.

**Decision gate.** At least 15% better time-to-durable on orion, within 5% of rsync,
without more than 5% regression on mars, bounded memory on the 4 GB Pi, or any loss of
resume correctness. Add one gate the original omitted: **no deadlock under the full test
suite run serially and in parallel, ten consecutive times**, since the failure mode here
is a hang rather than a wrong answer.

**Failure tests.** Kill the receiver with jobs in every stage; restart and prove that only
durable, journalled ranges are skipped. Inject a write error and full disk while later
chunks are already buffered. Run the resume path explicitly -- it is the case that broke
the first attempt, and it currently has **no automated coverage on the pull side** (see
4.65).

**If R1 stalls on the protocol question, `--streams` remains the stopgap.** Per-session
windows and per-session writes mean concurrency across sessions overlaps disk with
network without any receiver outrunning its sender. On orion `--streams 8` reaches
93-105 MB/s against single-stream's 83.2. It costs eight SSH sessions, carries high
variance, and **regresses mars by 1.19x**, so it is a per-destination workaround and must
not become a default.

## R2 — Adaptive local concurrency instead of a platform constant

**Hypothesis.** The useful worker count is a joint property of OS, filesystem, device,
corpus shape, and cache state. A short calibration or conservative online controller can
land close to the best fixed arm without a hard-coded macOS cap or core-count rule.

The evidence is unusually strong: Linux cold copies kept improving to 16-32 workers even
on a four-core Pi, while APFS peaked around 4-8 and degraded at 32. Warm and cold results
on the same Linux host produced different curves.

**Experiment.** Compare three policies:

- current platform default;
- a short preflight probe using a bounded sample of representative file sizes;
- an online controller that starts conservatively and changes concurrency using queue
  wait, completion latency, and device utilization, never CPU count alone.

Sweep APFS internal-to-USB, Linux NVMe-to-USB, Linux NVMe-to-NVMe, and Windows
NVMe-to-SATA. Include congress, cb7, and the bounded large-file set under warm and honestly
cold conditions. Keep metadata and data concurrency independently controllable; a single
number may be the wrong model.

**Decision gate.** The selected policy finishes within 5% of the best fixed arm on every
cell and is never more than 10% slower than the current default. Calibration must cost
less than 2% of the full run or be cached by a key containing OS, filesystem, and device.

**Likely output.** A policy such as low metadata concurrency plus storage-depth-scaled
data concurrency, with an explicit fallback when device identity or workload size is too
small to calibrate.

## R3 — A verified local-copy primitive shootout

**Hypothesis.** The current userspace read/hash/write loop leaves material local
large-file bandwidth unused, but the best replacement differs by platform and by whether
the source and destination share a copy-on-write filesystem.

**Experiment.** In a standalone spike, compare:

- the current buffered loop;
- larger buffered reads with hash and write overlapped;
- macOS `fcopyfile`/`copyfile` and Linux `copy_file_range`;
- Windows `CopyFile2` or the closest supported native copy API;
- memory mapping as a control, not as the presumed winner;
- whole-file clone and range clone when supported.

Every arm must provide xsync's current integrity guarantee. A kernel copy followed by a
second verification read is not automatically a win; report device bytes read/written,
CPU, cache pollution, and time-to-durable so a two-pass arm cannot hide its cost in the
page cache.

Use a 4-6 GiB incompressible set, cb7's large-file subset, and both same-device and
cross-device routes. Measure against the device's sustained, not burst/SLC-cache, limit.

**Decision gate.** Integrate a platform primitive only if it is at least 20% faster than
the current verified path or sustains at least 80% of the slower device's measured
sequential limit, with no more than 10% extra physical I/O and no correctness downgrade.

## R4 — Windows per-file cost decomposition

**Hypothesis.** A small number of avoidable metadata or publication operations account
for much of xsync's gap to robocopy. The known candidate is the Windows preflight
`symlink_metadata` call used only to discover `FILE_ATTRIBUTE_SPARSE_FILE`, but the
larger staging/create/set-metadata/rename sequence needs measurement rather than analogy
to Unix.

**Experiment.** Capture ETW/WPA filesystem and process traces for xsync and
`robocopy /MT:16 /R:0 /W:0` on a symlink-free `congress-100k` run. Attribute per-entry
counts and elapsed service time for open/create, query attributes, set attributes,
security checks, rename, delete, and flush. Run Defender-on as the primary condition and
repeat with the benchmark directory excluded only to separate tool work from scanner
work.

Then A/B only the top measured candidates. First carry file attributes in the scan record
to remove the preflight stat. If publication remains dominant, spike alternative Windows
staging/publication sequences while retaining atomic replacement and failure cleanup.

**Decision gate.** Explain at least 70% of the xsync-versus-robocopy wall gap with named
operations. A merged change should bring xsync within 1.25x of robocopy on
`congress-100k`, or remove at least 25% of xsync wall time, while retaining symlink and
metadata correctness on cb7.

**Non-goal.** Eliminating the Windows-versus-Linux OS penalty. The experiment owns the
gap between xsync and the best comparable Windows tool, not the platform's fixed cost.

## R5 — Persistent index v2: compact, on-disk, and loss-safe

**Hypothesis.** A memory-mapped, path-interned, sorted index can meet the original
O(changes) planning goal without the first prototype's 1.09 GB resident set and 2.70 s
planner.

**Experiment.** Compare representations before integrating any watcher:

- flat records sorted by parent id and basename, with a string table;
- a path trie/radix representation;
- an embedded on-disk B-tree or LSM design read through mmap;
- a compact fingerprint array plus append-only change log.

For each, measure cold open, warm open, full build, incremental update, lookup/merge plan,
on-disk size, peak RSS, and corruption recovery on `congress-1m`. The index must include a
schema version and source identity; a partial write must be recoverable without trusting
the index.

Only after a representation passes should a change feed be attached. FSEvents,
inotify/fanotify, and the Windows USN journal are hints. Overflow, dropped-event, journal
wrap, and unclean shutdown must force a bounded rescan, and the result must be compared to
a full independent walk.

**Decision gate.** Under 512 MiB peak RSS, preferably under 256 MiB; under one second to
produce a plan with less than 1% churn after warm open; exact agreement with a full walk
after a forced 40,000-file event loss. If no representation passes, keep one-shot scans
and do not build the daemon around a weak index.

## R6 — Sparse-aware local transfer first

**Hypothesis.** Extent-aware local copying is a low-network-dependency, categorical win:
the 3.7 TB apparent / 130 GB allocated VM image should cost approximately its allocated
bytes, not its logical size.

**Experiment.** Enumerate data and hole ranges using `SEEK_DATA`/`SEEK_HOLE`, macOS APIs
where their semantics differ, and `FSCTL_QUERY_ALLOCATED_RANGES` on Windows. Recreate
holes using seeks or native sparse-range controls. On a copy-on-write filesystem, compare
whole-file and range reflinks before falling back to extent reads and writes.

Test the real 17,145-extent image plus generated fixtures containing alternating 4 KiB
extents, a multi-gigabyte extent, holes at both ends, an all-hole file, and a filesystem
that reports no hole support.

**Decision gate.** Destination logical content matches the oracle; allocated size is
within 5% of the source; transferred/written data is within 10% of allocated bytes; the
current impossible corpus completes on available storage. Unsupported filesystems must
warn and fall back to dense transfer, never silently omit data.

**Reporting change.** Treat logical, allocated, physically copied, and wire bytes as four
different quantities. Sparse throughput is meaningless if those are collapsed.

## R7 — APFS clone-and-swap generations

**Hypothesis.** For local mirrors on APFS, cloning the complete source tree to a staged
generation and atomically exchanging it with the destination can beat per-entry planning
and mutation over a wider churn range than today's "destination absent" directory-clone
fast path.

This asks a different question from maximal absent-subtree cloning, which is already
implemented. It tests whether the destination should be treated as a replaceable
generation rather than a tree to edit in place.

**Experiment.** For a destination containing the previous generation:

1. clone the source root to a sibling staging path;
2. apply any metadata clonefile does not preserve;
3. verify the staged root;
4. atomically rename/exchange it into place;
5. retire the old generation after commit.

Compare with normal incremental sync at 0%, 0.1%, 1%, 10%, and 100% churn on
`congress-100k`, cb7, and a directory-heavy synthetic twin. Record temporary allocated
space, clone time, verification time, directory mtimes, open-handle behavior, and crash
recovery at each phase.

**Decision gate.** A clear churn/shape region where generation replacement is at least
1.5x faster, temporary allocated space remains bounded by copy-on-write behavior, and a
crash leaves either the old or new complete generation. If externally visible inode
identity or open-handle semantics make replacement surprising, expose it only as an
explicit mode or reject the idea.

## R8 — Local content reuse before network delta transfer

**Hypothesis.** Cross-file duplication in build trees can avoid local reads and writes via
whole-file or range reflinks. That may justify a content index even while gigabit Ethernet
makes the network version hard to distinguish.

**Experiment.** Capture two pinned consecutive cb7 build snapshots. Measure, in order:

1. whole-file digest reuse across different paths;
2. fixed-block reuse;
3. FastCDC reuse over a chunk-size sweep;
4. reuse after renames, relinks, and incremental rebuilds.

Report unique-byte fraction, index build time and size, lookup CPU, false-match defense,
and the fraction of reusable bytes that the destination filesystem can actually reflink.
Then simulate a local copy plan; do not implement wire delta under this story.

**Decision gate.** Continue only if first-sync unique bytes are below 70% or
consecutive-build unique bytes below 20%, and the index plus lookup costs less than 10% of
the avoided I/O time. First test a whole-file digest index; CDC must beat that simpler
design materially.

## R9 — Stream planning into transfer without weakening failure semantics

**Hypothesis.** Classification can feed transfer incrementally and recover much of the
measured 10.5-19% scan/plan dead interval. This helps small-file latency now and should
matter more when a faster link shortens the data phase.

**Experiment.** Partition planning into independently valid directory-ordered segments.
Begin transferring a segment only after all source errors capable of invalidating that
segment have been surfaced. Keep deletes globally deferred until the entire source scan
and transfer succeed. Compare the first-byte time and total time to the materialized-plan
path on local, macOS-to-Linux, and Linux-to-Linux congress runs.

The hard control is a late scan failure. Inject unreadable entries, disappearing files,
and a source directory replaced by a symlink after early segments have published.

**Decision gate.** At least 8% total wall improvement on one real 100k-entry route or 50%
lower time-to-first-byte, with a written partial-success contract. If the desired contract
is still "any scan error mutates nothing," retain full planning; that requirement and
early publication are incompatible without staging the whole destination generation.

## R10 — Shape vectors and strategy prediction

**Hypothesis.** Size distribution, directory density, depth, compressibility, sparse
extent density, and share of bytes above the large-file threshold predict useful strategy
better than corpus names or core count.

**Experiment.** Make the harness emit a shape vector containing entry kinds, depth and
fanout distributions, size percentiles, bytes above the large-file threshold, sampled
compressibility, duplicate whole-file bytes, and sparse extent statistics. Add at least:

- incompressible small files;
- a deep `node_modules`-shaped tree;
- a Git working tree;
- a few-files-per-directory and a thousands-per-directory pair;
- pathological but valid names.

Use leave-one-corpus-out tests to predict clone policy, local worker range, compression
choice, and expected metadata-versus-byte bound. A heuristic is not supported if it only
explains the corpora from which it was derived.

**Decision gate.** Predict the best strategy arm within 10% wall time on an unseen corpus.
Otherwise keep user-visible controls and conservative platform defaults instead of an
opaque auto policy.

## R11 — Experiments reserved for 2.5/10 GbE

Do not infer these results from gigabit measurements. Run a link ladder at 1, 2.5, and
10 GbE with the same endpoints, direct addressing where possible, and raw TCP plus SSH as
anchors.

### R11.1 — Find the first new large-file ceiling

Measure one and several large files to tmpfs and to fast NVMe in both directions. Record
per-endpoint CPU by function, wire throughput, chunk service time, in-flight bytes, memory,
and time-to-durable. The goal is to distinguish cipher/core limits, protocol framing,
hashing, memory copies, sender reads, receiver writes, and queue depth.

Only after this profile should chunk size, unacknowledged byte window, parallel hashing,
or extra data streams be revisited. At gigabit, depth 4 and 16 are already
indistinguishable and BLAKE3 is hidden by the link.

### R11.2 — Re-evaluate one connection versus many

If one SSH connection becomes CPU- or flow-control-bound, compare multiple SSH
connections with multiple logical streams over one authenticated connection. QUIC or
TLS-over-TCP is justified by daemon lifecycle, connection reuse, cancellation, and
isolation first; it should not be presumed faster than SSH, which matched raw TCP on the
measured Linux pair.

### R11.3 — Recompute strategy crossovers

Repeat compressible, incompressible, and mixed corpora with compression levels 1 and 3,
plus no compression. Recompute when compression becomes sender-CPU-bound. Do the same for
CDC/dedup only if R8 passed its byte-availability gate.

**10 GbE decision gate.** Produce a bottleneck table by link speed and corpus. No transport
rewrite or new parallel data path is justified without a cell showing at least 20%
recoverable headroom and naming the resource that caps it.

## Ideas not worth reopening yet

- **A different hash or GPU hashing.** Removing hashing did not improve measured wall
  time, and BLAKE3 is required for the integrity contract.
- **`io_uring`.** The spike did not beat plain syscalls, and the receiver worker curve
  showed that syscall CPU was not on the critical path. Reopen only with a new Linux
  profile proving an irreducible syscall bottleneck.
- **More large-file window depth.** Four and sixteen chunks in flight performed the same
  at gigabit. A new link is the required changed condition.
- **Blindly increasing streams.** Extra streams hurt Windows small-file transfers and
  fast-disk large-file transfers. Their current win on orion diagnoses serialized writes;
  R1 should remove that reason.
- **SSH cipher tuning or replacing SSH for throughput.** On the controlled Linux pair,
  SSH and raw sockets were within a few percent. Revisit only after R11 identifies SSH as
  the measured ceiling on the faster link.
- **Directory-affine dispatch.** The original experiment was invalid, but the motivating
  queue-wait profile fell from 18.58% to 2.04% after unrelated clone-path work. The prize
  is currently too small.
- **Unconditional compression.** It is a 2.4x loss on cb7 under `rsync -az` and a 1.8x
  loss on the Pi sender. Sampling and skipping is the right shape; improve its evidence,
  not its premise.
- **Optimizing only transfer-time versus rsync.** On Linux, rsync may return with gigabytes
  dirty while xsync has already flushed. Time-to-durable is the comparison that should
  drive changes involving writeback or checkpoint policy.

## Suggested execution order

1. R0 canonical baseline and trustworthy phase accounting.
2. R1 receiver writer pipeline; it has a measured present-day gap and a clean control.
3. In parallel by platform: R2 adaptive local concurrency, R4 Windows attribution, and
   R6 sparse local transfer.
4. R3 verified local-copy primitives and R7 APFS clone-and-swap.
5. R5 persistent-index representations; attach change feeds only after a representation
   passes.
6. R8 capture the two build snapshots and run the dedup decision spike.
7. R9 phase overlap and R10 shape prediction after R0 makes their effects observable.
8. R11 immediately after the faster link is stable and independently characterized.

Every experiment should retain the project's existing discipline: interleaved arms,
per-run exit and oracle verification, a warmup discarded where demonstrated necessary,
median plus dispersion, effective rather than requested settings, topology and cache state
recorded, bracket controls for device drift, and no claim from a failed or incomparable
run.
