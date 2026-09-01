# OPTIMIZE.md — investigable small wins

Candidates found while diagnosing the XFS regression (see
`benches/results/mars-local-fs/README.md`). Each entry records the **evidence**
that suggests it, an **estimated size**, and **how to test it**. Nothing here is
a known win: they are leads, ordered by expected value per unit of risk.

Ground rules, learned the hard way on this project:

- Measure before and after, paired against `rsync -a`, arms interleaved in one
  session, timed to durability, landed file count verified per run.
- A syscall you removed is not a win until the wall clock moves. Several
  entries below are cheap to implement and may still measure zero.
- Prefer refuting a candidate to implementing it.

## Baseline

`strace -c -f`, congress subset of 40,389 files + 37,768 dirs, mars,
NVMe → SATA SSD, after the reflink fix. Counts only — `strace -c` timings are
distorted by ptrace overhead.

| | rsync | xsync |
|---|---|---|
| filesystem syscalls | 1,213,990 | 1,225,498 |
| **per file** | **30.1** | **30.3** |
| scheduling (futex, sched_yield, …) | 64 | 101,867–335,643 |

The two tools are already within 0.7% on filesystem syscalls. **The remaining
overhead is concurrency, not I/O**, so weight the candidates accordingly.

Where they differ, the differences mostly cancel, and two of them favour xsync
already — do not "optimise" these without a reason:

| | rsync | xsync |
|---|---|---|
| stat family | 390,857 | **274,919** |
| chmod / utimensat | 234,473 | **156,316** |
| open + close | **277,577** | 431,281 |
| llistxattr | **0** | 40,389 |

---

## O-1. Apply metadata to the write fd instead of the path

**Status:** best-evidenced candidate. Not yet implemented.

`strace` of a single small file publish shows four operations:

```
openat(".xsync.tmp.<64 hex>", O_WRONLY|O_CREAT|O_TRUNC|O_NOFOLLOW)
openat(".xsync.tmp.<64 hex>", O_RDONLY)          <-- second open
chmod(".xsync.tmp.<64 hex>", 0644)
rename(".xsync.tmp.<64 hex>", "f184.dat")
```

The second open is `filetime::set_file_mtime`, which opens the path and calls
`futimens(fd)` — glibc renders that as `utimensat(fd, NULL, …)`, which is why it
does not appear as a path-based call in a census.

`Sink::write_file_with_retry` already holds a writable fd when it calls
`apply_file_metadata`. Using `filetime::set_file_handle_times(&file, …)` and
`File::set_permissions` on that fd would remove **one open and one close per
file**, and turn the `chmod` path lookup into an fd operation. That is ~2 of
~30 filesystem syscalls per file.

*Test:* `crates/xsync-core/src/sink.rs`, `write_new_temp` +
`apply_file_metadata`. Keep metadata applied **before** the rename — publication
atomicity depends on the destination never being visible without its final
mode and mtime.

*Caveat:* the microbenchmark in `benches/results/mars-local-fs/pubbench.c`
**under-models this**. Its `xsync` variant used a single path-based `utimensat`,
where the real code pays an extra open and close. Its `fdmeta` variant is
therefore closer to the true saving than the `xsync`-vs-`fdmeta` delta suggests.
Fix the model before trusting it: add a variant that opens the file a second
time to set mtime.

## O-2. Reduce `sched_yield` volume in the local worker pool

**Status:** largest absolute count, mechanism not yet confirmed.

xsync issues 90,723–335,643 scheduling syscalls where rsync issues **64** for
the same work. The spread between two runs of the *same* binary on the same
corpus (101,867 on XFS, 335,643 on ext4) means this is contention variance, not
a fixed cost — which also makes it noisy to measure.

Source is `crossbeam-channel`, which spins and yields before parking.
`transfer_files` uses `bounded(options.queue_capacity)` with a single task
channel feeding all 24 workers, so every worker contends on one queue.

*Test, cheapest first:*
1. Sweep `queue_capacity` — a queue too small makes producers and consumers
   trade yields.
2. Send tasks in **batches** rather than one per message, so a worker takes N
   files per channel operation.
3. Only then consider a different queue shape (per-worker queues with stealing).

*Expected:* CPU time, not necessarily wall clock — this workload is largely
I/O-bound, so a large syscall reduction may measure zero. Worth an explicit
null result either way.

## O-3. Stop materialising every task up front

**Status:** likely the cause of an observed memory difference; perf effect
unknown.

`transfer_files` (`local.rs`) collects **all** tasks into a `Vec` before
starting any work, and each `FileTask` holds two cloned `FileEntry` values plus
an owned `PathBuf`. At 1.3M files that is a large allocation made before the
first byte moves, and it matches the "xsync uses a lot more memory than rsync"
observation from earlier testing.

*Test:* stream tasks into the channel from the plan iterator instead of
collecting. Measure peak RSS and wall clock. Note the plan already spills to
disk through the planning spool, so the streaming machinery exists.

*Risk:* medium — check nothing downstream depends on the task count being known
up front (progress totals, in particular).

## O-4. Clone via `FICLONE` directly instead of spawning `cp`

**Status:** correctness-adjacent cleanup; performance effect small after the
reflink fix.

`platform_clone_file` and `platform_clone_directory` shell out to
`cp --reflink=always`. Two costs:

- **fork + exec per clone**, plus a PATH search: the trace shows
  `/usr/local/sbin/cp` and `/usr/local/bin/cp` both returning `ENOENT` before
  `/usr/bin/cp` succeeds — three `execve` per spawn.
- **The real errno is discarded.** `cp` failure collapses to
  `ErrorKind::Unsupported`, so the code could not distinguish "this filesystem
  cannot reflink" from `EXDEV`. That ambiguity is exactly what hid the
  cross-device bug for as long as it hid.

After the reflink fix this runs once per session for the probe, plus once per
qualifying directory subtree, so the throughput win is small. The value is
diagnosability.

*Blocker:* the workspace sets `unsafe_code = "deny"` with a single documented
exemption (`clonefile_native`). A raw `ioctl(FICLONE)` needs either a second
exemption or a safe wrapper crate (`rustix`). Decide that before implementing.

*Cheap partial:* if the spawn stays, invoke `/bin/cp` by absolute path to skip
the PATH search.

## O-5. Skip `llistxattr` when it cannot inform anything

**Status:** real cost, but it buys something — do not remove blindly.

40,389 calls, exactly one per transferred file, from the dropped-guarantee
preflight (`sparse.rs`). It powers a genuine warning: files whose xattrs or ACLs
xsync would otherwise drop **silently**. rsync makes no such call, but rsync
also does not warn.

The paired `stat` this pass used to cost has **already** been eliminated — the
planning spool carries uid/gid/nlink through `SourceFingerprint::unix`, and
`note_dropped_metadata` uses it. The `listxattr` half remains.

*Test:* skip the probe when it cannot change the outcome — e.g. when the
destination filesystem cannot store xattrs at all, so the warning is
unconditional anyway. Measure on a corpus with no xattrs (the common case).

*Do not* simply delete it. Losing metadata without telling the user is the
failure mode the pass exists to prevent.

---

## Already refuted — do not re-propose

Measured and dead. Kept so the next enthusiastic attempt costs nothing.

| Idea | Result |
|---|---|
| Shorter staging filename (64-char BLAKE3 hex is wasteful) | 12.04 s vs 12.00 s — free |
| Path-based vs fd-based metadata, *as modelled* | within noise (but see O-1: the model was wrong) |
| Dropping file metadata entirely | ~1% |
| Fewer local workers on XFS | strictly worse: 24 → 133 s, 1 → 196 s |
| `rmapbt=1` explains the XFS gap | never tested; cause was the reflink bug |
| Publication sequence explains the XFS gap | refuted by microbenchmark |
| Second `stat` in the dropped-metadata preflight | already eliminated via the spool |

**Skipping the temp-file-and-rename dance** measured ~7% (11.11 s vs 12.00 s),
the largest single number in the microbenchmark — and is **rejected on
correctness**. It trades away atomic publication: a crash mid-write would leave
a truncated file at the real destination path. Recorded because the number is
tempting and someone will find it again.
