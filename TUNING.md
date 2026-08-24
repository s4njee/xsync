# xsync performance tuning: real-world corpora and research spikes

This document replaces synthetic benchmarking as the basis for optimization work. It
defines the three real corpora xsync is tuned against, records the measured baseline on
each, and specifies the research spikes that follow from those measurements.

Companion documents: [plan.md](plan.md) for the v1 design, [tasks.md](tasks.md) for
story-level acceptance criteria, and
[benches/results/story-8.1/DECISION.md](benches/results/story-8.1/DECISION.md) for the
release matrix that produced the findings below.

The work breakdown for everything below — epics, stories, and acceptance criteria written
so any agent can execute them — is in [TUNING-TASKS.md](TUNING-TASKS.md).

**Scope: this document is v2 work.** v1 ships the correct, verified engine; the corpora
and spikes here define the optimization pass that follows it. Corpora A–D and spikes
S1–S8 are v2 targets. Section 7 records what is deliberately deferred to v3.

---

## 1. Why the synthetic corpora are retired

The Epic 0 generator (`xsync-bench corpus`) produces deterministic, content-pinned
fixtures across seven classes. Those remain valuable as **correctness** fixtures — they
are reproducible, they pin edge cases such as zero-byte storms and non-UTF-8 names, and
the manifest oracle depends on them. They are no longer the basis for **performance**
claims, for three measured reasons.

**They are the wrong shape.** A synthetic flat corpus enumerates far faster than a real
tree because real trees are directory-open bound rather than entry bound. The f2 project
measured 473k entries/s on a synthetic flat corpus against 38–43k entries/s walking a
real home directory — a 10x gap that no amount of tuning against the synthetic number
would have exposed.

**They are the wrong scale.** Every Story 8.1 result was `smoke` tier: 513 items and
1.77 MB for `mixed`, 1,000 entries and 61 KB for `deep-small`. At that size process
startup is a large share of the measurement, and several cells produced MAD/median above
the 15% comparability policy purely from noise. The real corpora below are 14 GB / 1.3M
files, 27 GB / 117 files, and 42 GB / 205k files.

**They hide both wins and losses.** Compression showed a 713x wire reduction on the
synthetic `compressible` class, but every route was a LAN or a local pipe where wire
bytes were never the constraint, so the advantage never appeared in wall time. Meanwhile
the 10x CPU overhead on small files was visible but easy to dismiss at 61 KB of payload.
At real scale it is unmissable — see §3.

Synthetic classes stay in the tree, are documented as legacy for performance purposes,
and continue to gate correctness.

---

## 2. The corpora

All four are live artifacts on the development host, referenced **read-only and in place**
rather than copied. Corpus D is a live VM disk rather than a purpose-built image, which is
a deliberate trade: it costs reproducibility and a dense baseline, and buys a 28.6x
sparseness ratio and a fragmented 17,145-extent map that no fresh install would produce.
See its section for the consequences that must be accepted with it.

Nothing in the benchmark harness may use a corpus root as a destination. Each corpus is
pinned by an independent manifest digest recorded at first use; a run whose source digest
no longer matches is reported as invalid rather than silently compared against different
content.

### Corpus A — `congress`: text at extreme file count

`~/projects/csearchv2/congress/data` — 1,318,771 files, 14 GB.

| Property | Measured |
|---|---|
| Composition | ~78% JSON, plus XML, txt, some zip |
| Size distribution | mean 251 KB, but 31% under 1 KB and 71% under 16 KB; long tail above 1 MB |
| Compressibility | **8.6x** at zstd-3 (1,604,723 → 187,033 bytes sampled) |
| Shape | `data/<congress>/{bills,amendments,votes}/<type>/<item>/…`, deep and highly branched |

This is the many-small-files case that plan.md's core thesis targets, at a scale where
per-file overhead cannot hide. It is also the corpus that validates compression on real
text.

**Tiers** — pinned as whole subtrees so each keeps authentic directory shape:

| Tier | Path (relative to `congress/data`) | Files |
|---|---|---:|
| `congress-1k` | `100/bills/hconres` + `100/bills/hjres` | 1,076 |
| `congress-10k` | `100` | 11,280 |
| `congress-100k` | `118` | 109,615 |
| `congress-1m` | `.` (whole tree) | 1,318,771 |

Whole subtrees are used rather than strided file samples because directory-open cost is
a first-order effect and a strided sample would flatten it.

### Corpus B — `manga`: incompressible media at large file size

`~/Downloads/Manga` — 117 files, 27 GB, 116 of them `.cbz`.

| Property | Measured |
|---|---|
| Mean file size | ~235 MB |
| Compressibility | **1.00x — zstd-3 output was 473 bytes larger than input** |
| Shape | flat, 4 directories |

`.cbz` is a ZIP container, so the payload is already compressed. This is the negative
control for the compression sampling heuristic: the heuristic must decline to compress,
and the cost of sampling must stay small. It is also the pure large-file throughput and
chunking case, and the only corpus here that exercises resume over meaningful byte
counts.

### Corpus C — `cb7`: mixed Rust/Tauri project with a real build tree

`~/projects/cb7` — 204,577 files, 42 GB.

| Property | Measured |
|---|---|
| Dominant subtree | `reader/src-tauri/target` at 39 GB |
| Composition | 85,529 `.o`, plus `.rlib`/`.rmeta`/`.a`, `node_modules` JS/JSON, source |
| Size range | six orders of magnitude, from sub-KB JSON to a 560 MB `libreader_lib.a` |
| Compressibility, `.o` | **7.8x** (20,000,000 → 2,555,423) |
| Compressibility, JS/JSON | **4.3x** (9,886,032 → 2,278,877) |
| Redundancy | 11.7 GB in files >50 MB, of which **~4.0 GB is redundant** by size collision |

**Correction to the original assumption:** Rust build artifacts are *not* incompressible.
Object files compress 7.8x — they are full of zero padding, repeated symbol names, and
DWARF debug information. cb7 is therefore mixed in *size and count distribution*, not in
compressibility; corpus B is the only genuinely incompressible one.

cb7's more interesting property is **duplication**. `libreader_lib.*` appears 44 times
across build profiles, and `debug/libreader_lib.rlib` and `debug/deps/libreader_lib.rlib`
are byte-identical at 165 MB each. This is the corpus that justifies and validates
content-defined chunking with a destination-side chunk index — a capability rsync
structurally cannot match, because its delta only ever compares a file against the same
path.

### Corpus D — `docker-raw`: sparse VM disk image

`~/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw` — the live Docker
Desktop VM disk. Measured with Docker stopped:

| Property | Measured |
|---|---|
| Apparent size | 3,721.9 GB |
| Allocated | 130.2 GB — **3.50% of apparent** |
| Sparseness | **28.6x** |
| Data extents | 17,145 |
| Extent size | min 4 KiB, median 96 KiB, max 5.62 GB |
| Mean extent | 7.8 MiB |
| Full extent map walk | **1.02 s** |

**Why this corpus exists.** plan.md files sparse preservation as a deferred v1 limitation,
alongside xattrs and ownership — a nice-to-have. This file reframes it as a hazard. Synced
today, xsync reads 3.7 TB of mostly zeros, hashes all of it, and attempts to write 3.7 TB.
`Sink::prepare_large` calls `set_len`, which produces a sparse file on APFS, and then every
chunk write materialises the holes back into real blocks. There is nowhere on this network
that can receive it: 1.1 TiB free locally, 238 GB cross-volume, 614 GB on `mars`. The
failure mode is not "less efficient" — it is filling the disk and failing, after hours.

**The number that justifies S8 on its own:** the complete hole structure of a 3.7 TB file
is enumerable in **1.02 seconds**. Learning exactly which 130 GB matters costs a second of
metadata traversal, against reading 3.7 TB to discover the same thing.

**Why a used image beats a fresh one.** A newly installed VM has a handful of large,
contiguous holes. This one has 17,145 extents spanning 4 KiB to 5.62 GB — a 1.4-million-fold
range produced by months of real allocation and deletion. That is a far more demanding test
of an extent enumerator, and it is not reproducible synthetically.

**What it exercises beyond sparseness:**

- **Rewrite-in-place.** VM disks change in the middle constantly, which a whole-file
  strategy handles badly and content-defined chunking handles well. This is the strongest
  motivating corpus for S4.
- **Resume.** A 130 GB transfer is long enough to be interrupted in real life, which no
  other corpus here makes true.
- **Stable-read under churn.** A running VM mutates its disk while it is read, the only
  realistic exercise of `SourceReader`'s retry path and vanished-file handling.

**Consequences of using a live file, which must be accepted explicitly:**

- **No dense baseline exists.** `rsync -a` and current xsync both fail with `ENOSPC`
  everywhere. The result is "cannot complete" → "completes in N seconds", which is
  dramatic but is *not* a paired ratio, so it cannot satisfy the Epic 0 gate on its own.
  The gate-able comparison is **`rsync -aS` against sparse-aware xsync**, both moving
  ~130 GB. Record `rsync -a` as a failed row with its error, not as a timing.
- **The digest drifts.** Any use of Docker invalidates the pin. Capture the manifest
  digest with Docker stopped, and treat cross-session comparisons as invalid unless the
  digest still matches. The §5 drift check is mandatory here, not advisory.
- **Local routes only.** At 130 GB a single SSH repetition is roughly 17 minutes on a
  gigabit link; five repetitions across two methods is over three hours. Run corpus D on
  same-volume and cross-volume, and size any SSH row deliberately rather than by default.
- **Cost per cell.** Five repetitions across two methods moves ~1.3 TB. Schedule it
  deliberately; it is not a developer-loop corpus.
- **Safety.** This file *is* the user's Docker state — every container, image, and volume.
  It is source-only. The harness must never accept a corpus root as a destination, and for
  this corpus that rule is protecting live data rather than a fixture.

**Baseline to beat.** `rsync -aS/--sparse`, which exists precisely because this case bites.

---

## 3. Measured baseline, and what it says

`congress-10k` (11,280 files, 112 MB), local same-volume APFS, initial copy:

| | wall | user | sys | total CPU |
|---|---:|---:|---:|---:|
| `rsync -a` | 3.75 s | 0.32 s | 4.58 s | 4.90 s |
| `xsync` | 7.20 s | 7.52 s | 21.37 s | **28.89 s** |

xsync is 1.92x slower in wall time and burns **5.9x the CPU**, of which **21.4 s is
system time** — 1.9 ms of kernel time per file against rsync's 0.41 ms, a 4.6x gap in
syscall cost per file. And it does this while copying **zero bytes**: the run reports
`0 physical` because the APFS clone path engaged.

That last detail is the whole finding. With byte movement already eliminated, we are
still nearly twice as slow, so the remaining cost is entirely per-file syscall volume.
This is precisely the f2 §1 lesson — per-file `COPYFILE_CLONE` was 2.70x while a
tree-level `clonefile` was 22x, cloning identical bytes, the entire difference being
per-file overhead. *When you are doing nothing per item, doing it ten thousand times is
still the whole cost.*

Two other baselines, same corpus family:

| Case | `rsync -a` | `xsync` | ratio |
|---|---:|---:|---:|
| congress-10k, no-op re-sync | 0.86 s | **0.45 s** | 1.91x |
| single 206 MB `.cbz`, local | 0.35 s | **0.07 s** | 5.0x |

The no-op win is real but fragile: xsync still spends 2.02 s of system time against
rsync's 1.02 s and wins on wall time only through parallelism. On a slower device, or
under contention, that advantage can invert. The large-file win is structural (clonefile)
and safe.

**Conclusion that drives every spike below:** we do not have a bytes problem or a hashing
problem. We have a syscall-volume problem on the per-file path, and our only durable
advantages come from doing categorically less work rather than the same work faster.

---

## 4. Research spikes

Each spike states a hypothesis, the evidence motivating it, the experiment, and a
success criterion that is checkable on a named corpus. They are ordered by expected
return per unit of effort, not by ambition.

### S1 — Account for and reduce the per-file syscall budget

**Hypothesis.** xsync issues several times more syscalls per file than rsync, and a
large fraction are avoidable: a per-file 64 KiB buffer allocation regardless of file
size, a per-file clone attempt that must fail before fallback, deterministic temp-name
hashing, and a create/write/setattr/rename sequence where rsync fuses steps.

**Evidence.** 21.37 s system time for 11,280 files with zero bytes copied; 1.9 ms kernel
time per file against rsync's 0.41 ms.

**Experiment.** Instrument a syscall trace for both tools on `congress-1k` and produce a
per-file histogram by syscall. Attribute the delta. Then remove the cheap items: size the
read buffer to `min(file_size, 64 KiB)`, skip the clone attempt for files below a
measured threshold, and cache or elide the temp-path hash.

**Success criterion.** `congress-10k` initial copy system time within 1.5x of `rsync -a`,
and wall-clock ratio ≥ 0.9. This is the prerequisite for every other local claim.

**Corpus.** congress-1k for tracing, congress-10k and congress-100k for confirmation.

### S2 — Persistent index and change journal

**Hypothesis.** rsync's structural weakness is that it rebuilds the entire file list on
every run: O(tree) work regardless of how much changed. A daemon maintaining a live index
turns re-sync into O(changes), which is a categorical win rather than a constant-factor
one.

**Evidence.** f2 §3 measured a warm index at 25.2 ms against 300.3 ms for
`readdir`+`fstatat` — 11.9x. f2 §10 found real daily churn is a few hundred authored
files, not the ~891k a naive count suggested. On `congress-1m` a full enumeration is
1.32M entries; the changed set on any given day is nearly empty.

**Experiment.** Build a read-only index prototype over `congress-1m`: initial build cost,
steady-state memory, incremental update latency via FSEvents, and time-to-first-plan
against a cold `xsync` run. Do not integrate it yet — measure whether the plan can be
produced without walking.

**Success criterion.** Plan for `congress-1m` with <1% of files changed produced in under
one second, against the multi-minute full walk. Index update must survive the FSEvents
lossy path: f2 §5 showed FSEvents raises `MustScanSubDirs`/`UserDropped` under a 40,000
file burst, and a client that ignores those flags goes permanently stale. Treat a dropped
subtree as requiring rescan, and test that explicitly.

**Corpus.** congress-1m primarily; cb7 as the adversarial case, since a build churns
tens of thousands of files in seconds.

**Note.** The daemon this requires is already in the original requirements (systemd /
launchd / Windows tray). This spike is the reason it earns its place.

### S3 — Clone at the highest unchanged subtree, not per file

**Hypothesis.** The whole-tree clone fast path only fires on a fresh copy. Incremental
syncs fall back to per-file cloning, which f2 measured at 2.70x against 22x for
tree-level cloning of identical bytes.

**Evidence.** congress-10k initial copy already reports `0 physical` bytes yet is 1.92x
slower than rsync — per-file cloning does not rescue per-file overhead. The 206 MB
single-file case, where one clone covers everything, is 5.0x *faster*.

**Experiment.** With S2's index or a cheap pre-pass, identify maximal subtrees that are
wholly unchanged or wholly absent at the destination, and clone at that root. Measure the
crossover in subtree size below which per-file work wins.

**Success criterion.** `congress-100k` with a single changed subtree completes in time
proportional to the changed subtree, not the whole tree.

**Corpus.** congress-100k and cb7 (whose `target/` contains large wholly-unchanged
subtrees between builds).

### S4 — Content-defined chunking with a destination chunk index

**Hypothesis.** A content-addressed chunk index on the destination makes renames, copies,
and cross-file duplication free. rsync cannot do this: its delta compares a file only
against the same path.

**Evidence.** cb7 holds 11.7 GB in files over 50 MB with ~4.0 GB redundant by size
collision, and `debug/libreader_lib.rlib` and `debug/deps/libreader_lib.rlib` are
confirmed byte-identical at 165 MB each. A rebuild rewrites artifacts that are largely
unchanged.

**Experiment.** FastCDC over cb7's `target/`, building a chunk index, and measure unique
versus total chunk bytes across two consecutive builds. This is a measurement spike
first: quantify the available win before implementing transfer.

**Success criterion.** Demonstrated unique-byte fraction below 70% on a first sync of
cb7, and below 20% across two builds. Below that, the complexity is not justified and
delta should stay deferred as plan.md already assumes.

**Corpus.** cb7 exclusively; congress and manga have little duplication.

### S5 — Compression policy against real data

**Hypothesis.** The adaptive zstd sample-and-skip heuristic is correct but has only been
validated on synthetic uniform corpora, where every file in a corpus shares one
compressibility. Real trees are mixed *within* a directory.

**Evidence.** Measured here: congress 8.6x, manga 1.00x (output larger than input), cb7
`.o` 7.8x and JS/JSON 4.3x. The original assumption that build artifacts are
incompressible is wrong, which means an extension-based heuristic would have been wrong
too — matching f2 §9, where sampling chose correctly without ever looking at a filename.

**Experiment.** Per-file sampling decisions across all three corpora: false-positive rate
(compressing incompressible data) and false-negative rate (skipping compressible data),
plus the sampling overhead as a share of wall time. Include the cb7 case where adjacent
files differ sharply.

**Success criterion.** <2% wall-time overhead on manga, >90% of achievable ratio captured
on congress and cb7, and no per-file decision worse than the whole-corpus optimum by more
than 5%.

**Corpus.** All three; manga is the decisive negative control.

### S6 — Parallelism shape: serial metadata, parallel data

**Hypothesis.** xsync parallelizes the wrong half. Ten workers on metadata-bound work add
contention without throughput.

**Evidence.** f2 §2 measured eight threads of `renameat` moving 13k/s to 14k/s because
APFS serializes directory metadata mutation, while f2 §1 measured parallel copying at
2.43x on the same machine. Our congress-10k run shows 7.52 s of user time and 21.37 s of
system time across 10 workers for work that moved no bytes.

**Experiment.** Sweep worker count from 1 to 16 on congress-10k and congress-100k,
separately for the metadata phase and the data phase, on both APFS and ext4.

**Success criterion.** A documented policy — likely a small fixed metadata concurrency
with data concurrency scaled to device queue depth — that beats the current uniform
worker pool on congress-100k without regressing manga.

**Corpus.** congress-100k for metadata, manga for data, cb7 for the mixed case.

### S7 — Measure where the advantages actually live

**Hypothesis.** The current matrix cannot show xsync's real advantages, because every
route is a LAN or local pipe with a warm cache, and both are regimes where our wins do
not apply.

**Evidence.** Compression delivered a 713x wire reduction that produced no wall-time
advantage, because the link was never the constraint. f2 §11 records that "cold" in these
tables means first repetition, not a cold kernel cache; f2 §7 shows a filesystem
conclusion that reversed entirely once Wi-Fi was replaced with Ethernet.

**Experiment.** Extend the harness with a bandwidth-limited route (traffic shaping to
50/100/1000 Mbit with injected latency) and a genuine cold-cache mode. Re-run congress
and manga on both.

**Success criterion.** A published table showing the link speed below which compression
and dedup dominate, and the speed above which syscall cost dominates. Every future
performance claim cites which regime it belongs to.

**Corpus.** congress (compressible, many files) and manga (incompressible, large files)
are the two extremes and bound the answer.

### S8 — Sparse-aware transfer

**Hypothesis.** xsync currently has no concept of a hole. For a sparse image it reads,
hashes, transfers, and materialises every zero byte, turning a 130 GB file into a 3.7 TB
write. Transferring only allocated extents is both a correctness fix and the largest
single throughput win available on this corpus.

**Evidence.** Corpus D measures 3,721.9 GB apparent against 130.2 GB allocated — 28.6x
write amplification and a guaranteed out-of-disk failure on every destination available
(1.1 TiB local, 238 GB cross-volume, 614 GB on `mars`). Decisively, its complete extent
map — 17,145 extents — walks in **1.02 s**, so the information needed to skip 96.5% of the
work is already essentially free to obtain.

**Experiment.** Enumerate allocated extents with `SEEK_HOLE`/`SEEK_DATA` (portable across
APFS, ext4, btrfs, XFS; Windows needs `FSCTL_QUERY_ALLOCATED_RANGES`). Transfer only data
extents, and reproduce holes at the destination by seeking rather than writing zeros.
Confirm the destination's *allocated* size matches the source's, not merely its apparent
size. The extent distribution is the real test: sizes span 4 KiB to 5.62 GB, so the
enumerator must handle both a 5.6 GB single extent and thousands of single-block extents
without degenerating into per-block I/O.

**Success criterion.** Corpus D transfers ~130 GB rather than 3.7 TB; destination allocated
size within 5% of source; and a transfer that currently cannot complete on any available
volume completes. Detection must degrade safely: a filesystem that does not report holes
falls back to dense transfer with a recorded warning, never to silent truncation.

**Baseline.** `rsync -aS`, which completes at ~130 GB and is the only honest paired
comparison. `rsync -a` and current xsync are recorded as failed rows with their `ENOSPC`
errors — a "cannot complete" → "completes" result is the headline, but it is not a paired
ratio and cannot satisfy the Epic 0 gate by itself.

**Corpus.** D exclusively. Corpora A–C have no holes and must show no regression.

**Note.** This spike changes what "bytes transferred" means in reporting. The event schema
already distinguishes logical, physical, and wire bytes; sparse transfer needs allocated
bytes recorded as a fourth quantity, or the throughput figures for corpus D will be
meaningless in both directions.

---

## 5. Harness changes this implies

- **Corpus registry.** The runner needs named real corpora with pinned digests and
  paths, alongside the legacy generator. Corpus roots are source-only; the runner must
  refuse to use one as a destination.
- **Oracle cost at scale.** Verifying a 1.32M-file destination after every repetition
  means hashing 14 GB fifteen times for a five-repetition three-method cell. The oracle
  needs either a sampling mode for the largest tiers or per-run reuse of a verified
  digest, with the mode recorded in the report so a sampled verification is never
  presented as a full one.
- **Drift detection.** A live tree can change between runs. Record the source manifest
  digest per run and fail comparison when it differs, rather than reporting a speedup
  between two different corpora.
- **Phase-level timing.** S1 and S6 need scan, plan, transfer, and metadata phases timed
  separately. The report schema already carries `phases_seconds`; the runner currently
  populates only seed/transfer/verify.

---

## 6. What is deliberately not on this list

**Multi-stream tuning.** f2 §6 measured framing at 20–80x against a per-file protocol,
and parallel streams at only 1.0–1.6x on top of framing. Story 8.1 reproduced the framing
half of that independently: batching took `deep-small` over SSH from 8.731 s to 0.343 s.
Stream count is a tunable, not an architecture concern, and further multi-stream work
should wait until S1, S2, and S3 are done.

**Beating rsync on a fresh copy of small files over a fast link.** Both tools are
syscall-bound there and rsync is near the floor. S1 exists to close the embarrassing part
of that gap, not to win it. The honest claim is parity on that workload, with wins on
incremental re-sync, on large files, on constrained links, and on duplicated trees.

---

## 7. Deferred to v3

These are real, measured gaps. They are listed so they are not rediscovered as surprises,
and explicitly held out of the v2 optimization pass.

### A real home directory — pathological metadata at scale

The case that breaks assumptions rather than throughput. A live home directory contains
sockets, fifos, and device nodes that xsync classifies as `other` and fails; hardlink-dense
trees such as a pnpm store or a Homebrew Cellar, where apparent bytes vastly exceed real
bytes and v1's lack of hardlink preservation inflates the destination; symlink density an
order of magnitude beyond anything synthetic; and names that coexist on a case-sensitive
Linux source but collide on case-insensitive APFS. It is also where f2 measured real trees
enumerating at 38–43k entries/s against 473k/s for a synthetic flat corpus — the 10x gap
that motivated retiring synthetic performance corpora in the first place.

Deferred because it is a correctness-and-semantics corpus more than a tuning corpus: most
of what it exposes needs v1 limitations lifted (hardlinks, special files, ownership) rather
than optimization, and it cannot be pinned by digest the way A–D can.

### Cloud placeholders and APFS-compressed files

Probing this machine's iCloud Drive (51,183 files) found files carrying both the `dataless`
and `compressed` flags — two distinct hazards in one place. A dataless file's contents are
not on disk; reading it to hash it triggers a download, and f2 counted 367,618 such files
here, so a naive sync means hundreds of thousands of cloud fetches costing bandwidth and
battery rather than CPU. APFS transparent compression is a separate problem: data lives in
an extended attribute, so `stat` size disagrees with allocated size and a naive copy
inflates the file.

This one is uncomfortable to defer, because `CloudFilesPolicy` and the `cloud_placeholders`
event **already ship** with three modes, macOS-only detection, and zero real-world
validation — no synthetic corpus can produce a dataless file. The Photos Library
(281 GB, 55,365 files, with a live SQLite database inside the bundle) is the concrete
vehicle. Promote this ahead of the home directory if shipped-but-unvalidated behaviour
matters more than breadth.
