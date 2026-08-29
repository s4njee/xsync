# BENCHMARK v2

Measured results for `xs` (xsync 0.1.0). This began as a single network
comparison against rsync, tar-over-ssh and scp, and has accumulated further
studies as questions arose; each is dated and self-contained, and later ones
sometimes correct earlier ones.

| Study | Machines | What it answers |
|---|---|---|
| [Baseline: congress/118 → mars](#baseline-congress118--marslocal) | mac → mars, 1 GbE | xsync vs rsync/tar/scp over a network |
| [`--streams` on large files](#--streams-on-large-files-corpus-b-manga--2026-08-28) | mac → mars | Where parallel streams pay, and where they do not |
| [Fix and re-measurement on cb7](#fix-and-re-measurement-corpus-c-cb7--2026-08-28) | mac → mars | The small-file `--streams` regression, fixed (2.70x) |
| [Cross-NVMe on freya](#cross-nvme-local-transfer-on-freya-corpus-a--2026-08-28) | freya, local | First local NVMe-to-NVMe; warm caches |
| [Cold cross-device NVMe](#cold-cross-device-nvme-congress-1m-on-freya--2026-08-29) | freya, local | congress-1m cold; ZFS vs ext4 reads; QLC variance |
| [Core-count heuristic refuted](#the-core-count-heuristic-is-wrong-on-small-machines--orion-pi-5-2026-08-29) | orion (Pi 5) | Whether worker count should track logical cores |

**Two corrections carried in later sections**, noted here so they are not missed
by anyone reading only the top: the `--streams` "contention" hypothesis in the
first findings section is wrong (it was an unpipelined code path), and the
12-worker scaling plateau reported on freya was a warm-cache artifact that does
not survive cold measurement.

## Baseline: congress/118 → mars.local

Wall-clock comparison of `xs` against rsync, tar-over-ssh with several
compressors, and scp, transferring the same corpus to the same host over the same
link.

## Dataset

Source: `corpora/congress/118` (congressional bills, amendments, and votes —
`data.json` / `data.xml` pairs, two files per leaf directory).

| Property | Value |
|---|---|
| **Total files** | **109,615** |
| **Total folders** | **25,851** |
| Total entries | 135,466 |
| Logical size | 556.9 MiB |
| On-disk size (APFS, allocated blocks) | 850 MiB |
| Mean file size | 5,327 B |
| Median file size | 2,117 B |
| p90 / max file size | 12,467 B / 3.0 MiB |
| Files under 16 KiB | 92.8% |

This is a **metadata-bound, many-small-files workload**, not a bandwidth-bound
one. The median file is ~2 KB and there are more directories than most corpora
have files. Per-file and per-directory syscall and round-trip costs dominate;
raw link bandwidth is nearly irrelevant. At 556.9 MiB the entire payload would
cross a saturated link in ~5 s, yet the fastest mode measured here takes 11 s and
the slowest takes 354 s.

## Results

Median of 3 runs unless noted. Sorted fastest to slowest.

| Mode | Runs | Median (s) | Range (s) | MiB/s | Files/s | vs. best |
|---|---:|---:|---:|---:|---:|---:|
| `rsync-az` | 3 | **11.0** | 9.8–13.0 | 51 | 9,992 | 1.00x |
| `rsync-a` | 3 | **14.6** | 13.0–16.4 | 38 | 7,513 | 1.33x |
| `xs-default` | 3 | **17.3** | 16.4–17.3 | 32 | 6,351 | 1.57x |
| `tar-zstd` | 3 | **26.4** | 25.9–29.4 | 21 | 4,157 | 2.40x |
| `tar-lz4` | 3 | **29.9** | 27.2–30.9 | 19 | 3,672 | 2.72x |
| `tar-plain` | 3 | **30.2** | 27.6–32.1 | 18 | 3,624 | 2.76x |
| `tar-zstd19` | 1 | **43.8** | — | 13 | 2,505 | 3.99x |
| `xs-streams8` | 3 | **211.5** | 209.7–226.1 | 3 | 518 | 19.28x |
| `xs-s8-nocomp` | 1 | **211.6** | — | 3 | 518 | 19.29x |
| `scp-r` | 1 | **353.5** | — | 2 | 310 | 32.23x |

MiB/s is computed on the 556.9 MiB logical payload, not on-disk blocks.

### Commands

| Mode | Command |
|---|---|
| `xs-default` | `xs --transport xsync SRC/ mars.local:DEST/ -q` |
| `xs-streams8` | `xs --transport xsync --streams 8 SRC/ mars.local:DEST/ -q` |
| `xs-s8-nocomp` | `xs --transport xsync --streams 8 --no-compress SRC/ mars.local:DEST/ -q` |
| `rsync-a` | `rsync -a SRC/ mars.local:DEST/` |
| `rsync-az` | `rsync -az SRC/ mars.local:DEST/` |
| `tar-plain` | `tar -cf - -C SRC . \| ssh mars.local "tar -xf - -C DEST"` |
| `tar-zstd` | `tar -cf - -C SRC . \| zstd -3 -T0 -c \| ssh mars.local "zstd -dc \| tar -xf - -C DEST"` |
| `tar-zstd19` | as above with `zstd -19 -T0` |
| `tar-lz4` | `tar -cf - -C SRC . \| lz4 -c \| ssh mars.local "lz4 -dc \| tar -xf - -C DEST"` |
| `scp-r` | `scp -rq SRC/. mars.local:DEST/` |

All `tar` modes ran with `COPYFILE_DISABLE=1 tar --no-mac-metadata` — see
[Correctness notes](#correctness-notes).

## Findings

### rsync wins, and compression is why

`rsync -az` is the fastest mode at 11.0 s. The `-z` flag is worth a consistent
25% over `-a` (14.6 s). These files are JSON and XML — highly compressible — and
rsync compresses the *stream*, so the per-file protocol chatter is compressed
along with the payload. On a workload this metadata-heavy, that chatter is a
large fraction of the bytes.

### Why `-axhP` is not the fastest

`-P` is the single biggest factor, but it is not the whole story — `-axhP` is
also missing `-z`. Isolating each flag against the same corpus, median of 3:

| Variant | Median (s) | vs. best |
|---|---:|---:|
| `rsync -az` | **11.07** | 1.00x |
| `rsync -az` (on a TTY) | 11.53 | 1.04x |
| `rsync -az --info=progress2` | 12.02 | 1.09x |
| `rsync -a` | 13.03 | 1.18x |
| `rsync -axh` | 13.25 | 1.20x |
| `rsync -azP` | 13.47 | 1.22x |
| `rsync -axhP` (on a TTY) | 15.47 | 1.40x |
| `rsync -axhP` (piped) | **16.28** | 1.47x |

Decomposed, `-axhP` is 5.21 s slower than `-az` (1.47x) for two roughly equal
reasons:

| Change | Cost |
|---|---:|
| Dropping `-z` | **+1.96 s** |
| Adding `-P` | **+3.03 s** |
| `-x` and `-h` together | +0.22 s (noise) |

- **`-x` and `-h` are free.** `-axh` (13.25 s) is statistically identical to `-a`
  (13.03 s). `-h` only formats printed numbers; `-x` adds a device check per
  directory. Neither is worth thinking about.
- **`-P` costs ~3 s because of output volume.** `--progress` emits roughly 144
  bytes per file — about **15.8 MB of progress text** across 109,615 files,
  versus literally 0 bytes for `-a`. `--partial`, the other half of `-P`, is
  effectively free when nothing is interrupted.
- **Dropping `-z` costs ~2 s** for the reason described above: rsync compresses
  the file list along with the payload.

**On the "printing to the screen" theory:** half right, with a caveat about what
this benchmark can and cannot see. Writing to a real PTY was *not* slower than
writing to a pipe here (15.47 s vs 16.28 s — the TTY run is nominally faster,
which is within run-to-run noise). So the ~3 s is rsync *generating and writing*
the progress text, not the terminal consuming it.

But that is a **lower bound on the interactive cost**. The PTY in this harness is
drained by a tight Python read loop that discards bytes. A real terminal emulator
must parse and render 15.8 MB with a carriage-return redraw per file, and that
work is not captured here. In an actual terminal the `-P` penalty is very likely
larger than 3 s — which is probably what you were seeing.

If you want progress without paying for it, `--info=progress2` prints a single
aggregate line that updates in place and costs **+0.95 s** rather than +3.03 s:

```
rsync -az --info=progress2 corpora/congress/118/ mars.local:dest/
```

### tar-over-ssh loses despite having no protocol overhead

The tar pipeline has no per-file negotiation at all — it is one continuous byte
stream — yet it lands at 2.4–2.8x slower than `rsync -az`. The bottleneck is
extraction: GNU tar on the receiving side creates 109,615 files and 25,851
directories serially, each with its own `open`/`write`/`close`/`utimes`. rsync's
receiver does the same work but overlaps it with the network transfer, while the
tar pipeline is limited by whichever end is slower.

Compressor choice barely matters (`zstd -3` 26.4 s, `lz4` 29.9 s, none 30.2 s)
because the pipeline is not bandwidth-bound. `zstd -19` is actively harmful at
43.8 s — the compressor becomes the bottleneck and buys nothing, since the link
was never full.

### `xs --streams 8` is a 12x regression over `xs` default

This is the most significant result and it reproduced cleanly across three runs
(209.7 s, 211.5 s, 226.1 s) against a stable 16.4–17.3 s for the single-stream
default:

| | Median | Files/s |
|---|---:|---:|
| `xs` (default, 1 stream) | 17.3 s | 6,351 |
| `xs --streams 8` | 211.5 s | 518 |

Adding parallel streams makes xsync **12.2x slower** on this corpus. Two things
narrow the cause:

- **It is not compression.** `--streams 8 --no-compress` is 211.6 s, statistically
  identical to `--streams 8` with compression. The regression is in the stream
  machinery itself, not the compressor.
- **It is not the worker pool.** The default run already reports `workers 10`, so
  xsync is doing concurrent local I/O in both cases. Only the number of network
  streams differs.

The shape of the number — throughput collapsing to ~518 files/s, close to a
per-file serialization floor — suggests contention or synchronization across
streams, with per-file work being serialized on a shared resource rather than
overlapped. Worth profiling; on this workload the flag is a pure loss.

> **Correction (2026-08-28): the contention hypothesis above is wrong.** It is
> not contention, and profiling was not needed to settle it. `--streams N > 1`
> dispatches to `sync_push_server_streams`, a separate implementation that
> predates the batching and pipelining work and never received it. Every
> pipelining call site in `server.rs` (`MAX_PIPELINED_FRAMES` / `drain_acks`)
> lives in the single-stream paths at lines 2974–4067; that function spans
> 5206–5958 and contains none of them, only twelve blocking `expect_ack` calls.
> Its small-file loop sends each file as its own one-entry `FileBatch` plus one
> `FileSegment`, each followed by a synchronous ack — and it does so on the
> **control** session, not the data sessions, which only ever carry large-file
> ranges. On a corpus of small files the extra streams do nothing at all except
> spawn N more `ssh` children.
>
> The decisive measurement is that **the cost does not vary with stream count**,
> which is the opposite of what contention predicts. congress-1k (1,076 files),
> mars.local, 1.287 ms mean RTT, median of 3:
>
> | Mode | Median | Files/s |
> |---|---:|---:|
> | default (1 stream) | 0.35 s | 3,074 |
> | `--streams 2` | 4.19 s | 257 |
> | `--streams 4` | 4.42 s | 243 |
> | `--streams 8` | 4.42 s | 243 |
>
> The entire 12x is paid going from 1 to 2 streams. Going from 2 to 8 costs 5%
> while the stream count quadruples. At 243 files/s the per-file cost is 4.1 ms,
> or roughly three round trips — consistent with two synchronous acks plus the
> source read, and inconsistent with any lock-contention story.
>
> This also explains the `--no-compress` result that looked so puzzling. The
> multi-stream control session advertises `capabilities=0x0` — no `CAP_ZSTD` —
> while its data sessions get `0x3` and the single-stream client gets `0xa`. All
> small-file traffic goes over the control session, so on this corpus
> compression was never enabled in either arm. There was nothing for
> `--no-compress` to turn off.
>
> **A second, separate bug:** `--streams 16` fails outright with
> `xs: I/O error: Broken pipe (os error 32)`. xsync opens one SSH connection per
> stream plus one for control, so 17 concurrent connections — above OpenSSH's
> default `MaxStartups 10:30:100` random-early-drop threshold. mars has no
> explicit `sshd_config` override, so the defaults apply; this is the likely
> cause but is unconfirmed, since `sshd -T` needs root there. Either way the
> failure is opaque and arrives after several sessions have already succeeded.
>
> **The fix** is not a new design: extract the batched small-file sender that
> `run_client_push` already has (server.rs:3633–3660, which coalesces up to
> `BATCH_TARGET_SIZE` and pipelines the acks) and call it from the multi-stream
> control session, leaving the data threads to do what they were built for —
> large-file ranges. Until then `--streams` is a pure loss on anything but a
> corpus of large files, and the default of 1 is the right default.

`xs` at its default settings is credible: 17.3 s, third overall, and the
tightest run-to-run spread of any mode (0.9 s across three runs, versus 3.2 s for
`rsync-az`). It beats every tar pipeline. It is 1.57x off `rsync -az`, and
closing that gap plausibly means compressing protocol chatter the way rsync's
`-z` does.

### scp is not a contender

353.5 s, 310 files/s. Modern `scp` runs over the SFTP protocol, which round-trips
per file; at 1.5 ms RTT and 109,615 files that overhead alone accounts for most
of the runtime. Included as a baseline for what per-file synchronous protocols
cost on this shape of data.

## Correctness notes

Every run was verified by counting files and directories at the destination.
All modes in the results table landed exactly **109,615 files / 25,850
directories** (the harness counts with `-mindepth 1`, excluding the destination
root itself, so 25,850 corresponds to the 25,851 figure above).

Two measurement bugs were found and fixed rather than reported:

1. **macOS `tar` inflated the first tar runs.** The initial `tar-plain`,
   `tar-zstd`, and `tar-lz4` runs each delivered **245,081** files instead of
   109,615 — Apple's `bsdtar` emits AppleDouble `._*` sidecars carrying extended
   attributes, and GNU tar on Linux extracts them as 135,466 additional real
   files — exactly one sidecar per entry (109,615 files + 25,851 directories). This inflated both the payload (1.5 GB vs 974 MB on disk) and the times
   (83.6 / 89.6 / 88.8 s). Re-run with `COPYFILE_DISABLE=1 tar
   --no-mac-metadata`, the tar modes land at 27.6 / 26.4 / 27.2 s. The discarded
   runs are excluded from the table. Note that the inflation is invisible if you
   list the archive with macOS `tar`, which re-merges the sidecars.

2. **`xs` would have silently benchmarked rsync.** mars.local had no `xs`, and
   `--transport auto` falls back to rsync when the remote binary is missing —
   which would have measured rsync twice under two names. A matching
   `x86_64-unknown-linux-gnu` binary was cross-built and staged via
   `scripts/deploy-mars.sh`, and the xsync transport was confirmed active by the
   `[xsync server]` handshake in the session log before any timing was recorded.

## Methodology

- **Cold destination.** The remote directory is `rm -rf`'d and recreated before
  every run, so every mode performs a full transfer with nothing to skip. No mode
  benefits from delta detection against a prior run.
- **Warm source.** The local page cache is primed with a full read pass before
  each pass, so no mode is charged for cold-reading 850 MiB off local disk.
  Timings therefore reflect transfer and destination-write cost, not source I/O.
- **Timing** via zsh `$EPOCHREALTIME` around the complete pipeline, including
  process startup and teardown.
- **Verification** after each run: destination file count, directory count, and
  `du -sk`, compared against the source.
- Runs were executed sequentially, never concurrently, to avoid contending for
  the same link.

### Environment

| | Source | Destination |
|---|---|---|
| Host | macOS 26.6.1, arm64 | mars.local — Arch Linux, kernel 7.1.6, x86_64 |
| Filesystem | APFS | ext4 on NVMe (`/dev/nvme0n1p2`, 860 GB free) |
| rsync | 3.4.4 (protocol 32) | 3.4.4 (protocol 32) |
| tar | bsdtar 3.5.3 / libarchive 3.7.4 | GNU tar 1.35 |
| zstd / lz4 | 1.5.7 / 1.10.0 | 1.5.7 / 1.10.0 |
| OpenSSH | 10.3p1, LibreSSL 3.3.6 | 10.4p1, OpenSSL 3.6.3 |
| `xs` | 0.1.0 (`3b3f488dcae2`), aarch64-apple-darwin | 0.1.0 (`3b3f488dcae2`), x86_64-unknown-linux-gnu |

**Link:** gigabit LAN, 1.5 ms mean RTT (min 1.325 / max 1.764 ms), 0% packet
loss. Measured ssh throughput ceiling ~105 MB/s (300 MB piped to `cat >
/dev/null` in 2.86 s). No mode came close to saturating it, confirming the
workload is metadata-bound rather than bandwidth-bound.

### Tools considered but unavailable

`bbcp`, `mbuffer`, `pigz`, and GNU `parallel` are not installed on either host;
`nc` and `socat` are absent on mars.local. Benchmarking those would have
required installing software on the remote, which would change the machine being
measured. The comparison is limited to what both hosts already had.

## Caveats

- `tar-zstd19`, `xs-s8-nocomp`, and `scp-r` are single runs. Their absolute
  numbers are less trustworthy than the 3-run medians, though all three are far
  enough from their neighbours that the ordering is not in question.
- Single corpus, single link, single direction. The many-small-files shape here
  is close to the worst case for per-file protocols; a corpus of large files
  would likely reorder the table substantially, and the tar pipelines in
  particular should do much better there.
- All destinations were empty. This measures cold full transfer only — it says
  nothing about incremental re-sync, which is rsync's and xsync's actual design
  point.

---

## `--streams` on large files (corpus B, Manga) — 2026-08-28

The congress result above says `--streams` is a 12x loss. That is a statement
about *small* files, and it does not generalize: on large files the flag does
what it was designed to do. Measured on the same link the same day, with the
single-ssh wire ceiling re-measured at **106 MB/s** (500 MB piped to
`cat > /dev/null`, median of 3).

Sources are APFS clones of `~/Downloads/Manga` files, staged so the corpus itself
is never read from twice or modified. Destination is ext4 under `~` on mars —
**not** `/tmp`, which is a 16 GB tmpfs there and would have been backed by RAM.

**Tier A — one 885 MB file.** Median of 3, all arms within the 15% MAD policy.

| Mode | Median | MB/s |
|---|---:|---:|
| 1 stream | 12.2 s | 73 |
| `--streams 2` | 10.1 s | 88 |
| `--streams 4` | **9.9 s** | **90** |
| `--streams 8` | 10.8 s | 82 |

**Tier B — 1,383 MB across 7 files.** Median of 3; spreads were exceptionally
tight (MAD/median ≤ 1.8% in every arm).

| Mode | Median | MB/s | Setup | Transfer only | MB/s |
|---|---:|---:|---:|---:|---:|
| 1 stream | 16.8 s | 82 | 0.19 s | 16.6 s | 83 |
| `--streams 2` | 14.9 s | 93 | 0.86 s | 14.0 s | 99 |
| `--streams 4` | **14.4 s** | **96** | 1.37 s | 13.0 s | **106** |
| `--streams 8` | 16.8 s | 82 | 1.30 s | 15.5 s | 89 |

"Setup" is measured separately by transferring a single 6-byte file at each
stream count, which isolates connection establishment: `spawn_server_child` is
called in a **sequential** loop, so N SSH connections are opened one after
another before any data moves.

### What the numbers say

**Single-stream xsync is not wire-limited; it is limited by its own pipeline.**
One `ssh` sustains 106 MB/s on this link, but single-stream xsync moves bytes at
83 MB/s — 78% of what the wire allows. The difference is xsync's own per-stream
work: source read, BLAKE3, framing, and the destination's write and verify.

**Four streams is exactly enough to reach the wire.** With setup subtracted,
`--streams 4` transfers at 106 MB/s, which is the measured ceiling to three
significant figures. That is the flag working precisely as designed — it
parallelizes xsync's own CPU-bound per-stream work until the network becomes the
constraint, and then stops helping because there is nothing left to win.

**Eight streams is worse than four**, and only about a third of that is setup
cost (1.30 s of a 2.4 s deficit). The remaining ~1.1 s is real degradation —
nine SSH connections competing for CPU and for a saturated link. Not
investigated further, because the headroom it would recover is zero: four
streams already has the wire.

**So the honest headline is ~1.2x, not more.** `--streams 4` is 1.17x faster
than one stream end to end on Tier B, and 1.23x on Tier A. That is the entire
size of the prize on gigabit, because 78% → 100% of the link is all there is to
claim. The flag would matter much more on a faster link, where a single stream's
83 MB/s pipeline would leave far more of the wire idle.

### A third bug: `--streams` is broken for single-file sources

```
xs --streams 4 /path/to/Berserk\ v38.cbz mars.local:~/dest/
xs: source read error: cannot open source '.../Berserk v38.cbz/Berserk v38.cbz':
Not a directory (os error 20)
```

The multi-stream path appends the entry's relative path to `source_path`, but for
a file source `source_path` is already the file, so the basename lands twice. The
single-stream path computes a `source_reader_root` for exactly this case
(server.rs:5372) — `run_data_thread` is handed the raw `source_path_buf` instead.
Every `--streams N > 1` transfer of a single file fails, after the data sessions
have already reported success.

---

## Fix and re-measurement, corpus C (cb7) — 2026-08-28

Two of the three defects above are fixed.

**1. Small files no longer cost a round trip each.** The batched, pipelined
sender is now a single function, `send_small_files_batched`, called by both
`run_client_push` and the multi-stream control session. It was duplicated before,
which is exactly how the two paths diverged; there is now one implementation to
keep correct. The multi-stream control session passes `compress: false`, because
it negotiates `capabilities=0x0` and sending zstd frames there would break the
receiver — a separate missed optimization, not fixed here.

**2. Single-file sources work.** `run_data_thread` receives `source_reader_root`
rather than the raw `source_path`, so a file source no longer produces
`<file>/<file>`. Verified end to end: an 885 MB file striped across 4 data
sessions arrives with a matching SHA-256.

`--streams 16` still fails with `Broken pipe`; that one is untouched.

### Corpus C has changed since TUNING.md was written

TUNING.md describes cb7 as 204,577 files / 42 GB. It is now **59,311 files plus
3,310 symlinks, 5.49 GiB** — the build tree has been cleaned since. The
composition is what makes it the most useful corpus for this question:

| Size bucket | Files | Share |
|---|---:|---:|
| < 1 KB | 28,591 | 47.4% |
| 1–8 KB | 21,226 | 35.2% |
| 8–64 KB | 7,828 | 13.0% |
| 64 KB–1 MB | 2,175 | 3.6% |
| 1–8 MB | 387 | 0.6% |
| > 8 MB | 78 | 0.1% |

**82.6% of files are under 8 KB**, and yet **68% of the bytes live in the 78
files over 8 MB**. So cb7 exercises the small-file control session and the
large-file data threads at the same time, which neither congress nor Manga does.

### Before and after, cb7, `--streams 8`

The "before" binary is a clean build of HEAD (`3b3f488d`) in a separate git
worktree, so it carries none of this session's changes.

| | Median | MB/s |
|---|---:|---:|
| before the fix | 149.3 s | 38 |
| after the fix | **55.3 s** | **102** |

**2.70x.** Worth noting that a projection from the congress per-file rate put the
"before" figure near 300 s; the measured value is 149 s. The projection was
wrong, which is why it was measured.

### Stream sweep on cb7, after the fix

Median of 3, all arms within 1.4% MAD/median.

| Mode | Median | MB/s | vs 1 stream |
|---|---:|---:|---:|
| 1 stream | 58.6 s | 96 | 1.00x |
| `--streams 2` | 62.2 s | 90 | **0.94x** |
| `--streams 4` | 57.0 s | 99 | 1.03x |
| `--streams 8` | **55.3 s** | **102** | 1.06x |

### What this means for `--streams`

**The flag is no longer catastrophic, and it is still not worth using on mixed
workloads.** 1.06x at eight streams, and two streams is a 6% *regression*. The
default of 1 remains correct.

The three corpora now tell a consistent story about where the flag pays:

| Corpus | Shape | Best mode | Gain |
|---|---|---|---:|
| congress-1k | all small files | 1 stream | — |
| cb7 | 83% small files, 68% of bytes large | `--streams 8` | 1.06x |
| Manga | all large files | `--streams 4` | 1.17–1.23x |

The gain tracks the share of bytes going through the data threads, which is what
the design predicts. On cb7 single-stream already reaches 96 MB/s of a 106 MB/s
wire ceiling, so there was only ~10% of headroom to win in the first place.

**One caveat on all of this:** 1 stream and N > 1 streams are different code
paths, not the same path with a different width. These tables compare two
implementations, not the effect of parallelism in isolation.

---

## Cross-NVMe local transfer on freya (corpus A) — 2026-08-28

The first measurements on a second Linux host, and the first that are local
rather than networked: NVMe to NVMe, across two devices and two filesystems.

**Host.** freya, AMD Ryzen 9 7950X (32 threads), 61 GB RAM, kernel 7.1.1-cachyos.

| | Device | Filesystem |
|---|---|---|
| Source | `nvme0n1` SHGP31-2000GM | ZFS `zpcachyos`, **compression=lz4**, recordsize 128K, atime=off |
| Destination | `nvme1n1` SOLIDIGM SSDPFKNU020TZ | ext4, `rw,relatime` |

**Methodology, and why it needed care.**

- `sync()` is inside the timed region. Without it the ext4 side is page cache and
  the benchmark measures RAM.
- The source is 557 MB against 61 GB of RAM with ZFS ARC allowed to reach
  60.9 GB, so **reads are warm**. These numbers measure the tool's per-file
  efficiency, not NVMe read bandwidth. Dropping caches needs root, which is not
  available unprivileged here.
- **freya is not a quiet machine.** It runs k3s, containerd and netdata; load
  average during this work ranged from 3 to 11, and ARC was reclaimed from
  12.7 GB to 4.3 GB mid-session. An early sequential-block sweep produced
  14–20 s outliers at 16/32/48 workers that vanished on re-measurement — they
  were background load, not the worker count. Every comparison below is either
  5 reps with MAD reported, or **interleaved A/B pairs**, which is the only
  design that survives drift on a host like this.

### The finding: this session's own preflight cost 16%

The phase breakdown showed 0.34 s of a 2.26 s run — 15% — between "plan end" and
"transfer start". That window is where the V3.1 collision check and the V3.3
dropped-metadata preflight run. Interleaved A/B, 6 pairs, 16 workers:

| | Median |
|---|---:|
| preflight on | 2.25 s |
| preflight off | 1.88 s |

**16.4%**, on a pass that transfers nothing and only produces a warning. The
V3.3 preflight is two syscalls per transferred file — a `stat` and a
`listxattr` — 219,230 of them here, run serially on one thread while 16 workers
sat idle. (The collision check measured ~1%; it is not the problem.)

### The fix: parallelize it

`sparse::inspect_with_workers` splits entries into contiguous chunks across the
same worker count the transfer uses, and merges in chunk order — so the result is
identical to the serial version regardless of scheduling, including the hardlink
accounting, which depends on which name for an inode is seen first. A test builds
a fixture with hardlink groups deliberately spanning chunk boundaries and asserts
the parallel result equals the serial one at 2, 3, 8 and 16 workers.

Interleaved A/B after the change, 8 pairs:

| | Median | MAD |
|---|---:|---:|
| preflight on | 1.94 s | 1.8% |
| preflight off | 1.92 s | 1.3% |

**16.4% → ~1%.** The pass still does every syscall; it just stops doing them on
one thread.

### Worker scaling, congress-100k, ZFS -> ext4

5 reps each, all arms MAD ≤ 1.5%.

| Workers | Median | vs 1 worker |
|---:|---:|---:|
| 1 | 5.52 s | 1.00x |
| 2 | 3.80 s | 1.45x |
| 4 | 2.83 s | 1.95x |
| 8 | 2.36 s | 2.34x |
| 12 | 2.22 s | 2.49x |
| 16 | 2.22 s | 2.49x |
| 24 | 2.21 s | 2.50x |
| 32 (default) | 2.22 s | 2.49x |

**Scaling stops dead at 12 workers on a 32-thread machine.** Everything from 12
to 32 is the same number. The default of one worker per logical core is not
harmful, but 20 of those 32 threads contribute nothing — at 557 MB / 2.2 s the
run is bound by per-file work, not bandwidth. NVMe is nowhere near saturated.

### Head to head, after the fix

congress-100k, 5 reps, `sync()` included.

| Tool | Median | MAD | vs xs |
|---|---:|---:|---:|
| `xs --local-workers 16` | **1.95 s** | 1.5% | 1.00x |
| `xs` (default, 32 workers) | 2.01 s | 0.5% | 1.03x |
| `tar c \| tar x` | 2.01 s | 0.5% | 1.03x |
| `rsync -a` | 3.43 s | 0.6% | 1.76x |
| `cp -a` | 5.89 s | 1.5% | 3.02x |

xsync is **1.76x faster than rsync** and **3.0x faster than cp** on cross-NVMe.

The result worth keeping in view is `tar`: a single-threaded pipe matches xsync's
32-worker default to within noise. Before the preflight fix, tar was *ahead*
(1.97 s vs 2.22 s). Whatever the remaining serial bottleneck is — it caps
scaling at 12 workers and leaves a 32-thread machine idle — a plain sequential
stream reaches the same place without any of the machinery. That is the next
thing to find, and it is worth more than any further worker tuning.

---

## Cold cross-device NVMe, congress-1m on freya — 2026-08-29

The previous freya section measured congress-100k with warm caches. This one
uses the full corpus — **1,318,771 files, 468,775 directories, 11 GB logical** —
with caches dropped before every rep, which changes several of its conclusions.

### Hardware, read from the machine

| | Model | Link | Filesystem |
|---|---|---|---|
| `nvme0n1` | SK hynix SHGP31-2000GM (Gold P31 2TB, TLC) | PCIe 3.0 x4 | ZFS `zpcachyos`, lz4 |
| `nvme1n1` | SOLIDIGM SSDPFKNU020TZ (P41 Plus 2TB, **QLC**) | PCIe 4.0 x4 | ext4 `/mnt/nvme` |
| `sdq2` | USB 3.1 enclosure, 10 Gbps link | ~1 GB/s | ext4 `/mnt/usb` |

Measured sequential: internal 3.1 GB/s write / 3.6 GB/s read; USB 979 / 882 MB/s.

### Warm caches were hiding more than half the work

Same config (16 workers, ZFS -> ext4), only the cache state differs:

| | Time | Files/s | MB/s |
|---|---:|---:|---:|
| cold | 101.6 s | 12,980 | 108 |
| warm | 44.6 s | 29,570 | 247 |

The corpus is 9.3 GB on ZFS against 61 GB of RAM, so warm runs were largely
RAM-to-device. Every number below is cold.

### The variance was the QLC drive, not ZFS

An uncontrolled sweep gave 61-94 s for one config. The pattern points at the
destination, not the filesystem:

- writing to the **QLC** Solidigm: 61-94 s swings, several arms breaching the
  15% MAD policy;
- writing to the **TLC** Hynix through ZFS: 164.61/164.81, 153.23/154.19,
  149.51/150.47 — MAD 0.1-0.3%.

A QLC drive's dynamic SLC cache makes write speed depend on recent history, which
is exactly the signature seen. This is a correction: an earlier note in this file
attributed the swings to ext4 writeback scheduling.

**The USB drive was then checked rather than assumed** — 64 GB written in 4 GB
steps held 979-984 MB/s throughout, with a single 842 MB/s dip. No SLC cliff
within 64 GB, so it is a clean write target and the ZFS -> USB configuration
below isolates xsync from destination cache state.

> **The drive is an Inland Premium 256 GB** (M.2 2280, PCIe NVMe 3.0 x4,
> **TLC** 3D NAND, rated 2,900 MB/s read / 950 MB/s write). The TLC rating
> explains the flat sustained-write result directly — there is no QLC SLC cliff
> to hit.
>
> It also corrects an attribution made above: writes were **drive-limited, not
> link-limited**. 979 MB/s measured against 950 MB/s rated is the drive, not the
> 10 Gbps enclosure. Reads were link-limited (882 MB/s measured against 2,900
> rated). The bottleneck therefore differs by direction and by host:
>
> | Host | USB link | Write | Read | Limited by |
> |---|---|---:|---:|---|
> | freya | 10 Gbps | 979 MB/s | 882 MB/s | drive on write, link on read |
> | orion (Pi 5) | 5 Gbps | 357 MB/s | 322 MB/s | link, both directions |
> | this Mac (M1 Max) | 10 Gbps | 913 MB/s | — | drive on write |

### Worker scaling, cold, congress-1m

3 reps each, caches dropped before every rep, `sync()` inside the timed region.

| Workers | internal ext4 -> USB | USB -> internal ext4 | ZFS/TLC -> USB |
|---:|---:|---:|---:|
| 8 | 67.3 s (2.0%) | 104.2 s (0.0%) | 111.8 s (4.2%) |
| 16 | 55.4 s (1.0%) | 81.5 s (0.6%) | 90.1 s (0.8%) |
| 32 | **53.6 s** (0.3%) | **70.1 s** (2.0%) | **77.9 s** (3.6%) |

Parenthesised figures are MAD/median. Every arm is comparable — ext4-to-ext4
repeatability is far better than anything with ZFS or the QLC drive in the write
path.

**Correction: cold scaling does not plateau at 12 workers.** The warm 100k
section concluded that 12, 16, 24 and 32 workers were indistinguishable, and a
backlog story was filed to hunt the serial bottleneck responsible. Cold, that
plateau does not exist — throughput improves monotonically to 32 in all three
configurations. With a warm cache there is no I/O to wait on, so the run is CPU-
and lock-bound and extra threads have nothing to hide; cold, each file carries
real device latency and more workers hide more of it. The plateau was an artifact
of measuring with warm caches, and the default of one worker per logical core is
right for real transfers.

### Does the core count actually set the optimum?

freya has 32 logical cores and cold scaling was still improving at 32, so the
obvious reading is "use `available_parallelism()`". But 32 was also the largest
value tested, and core count is a CPU property while the mechanism — hiding
per-file device latency — is a device property. Extending the sweep past the core
count (`--local-workers` caps at 64), cold, internal ext4 -> USB, 3 reps:

| Workers | Median | MAD |
|---:|---:|---:|
| 32 | 54.9 s | 0.7% |
| 48 | 54.2 s | 1.7% |
| 64 | 55.5 s | 2.3% |

**Indistinguishable.** Scaling genuinely plateaus at ~32, and oversubscribing to
2x the core count neither helps nor hurts. So one worker per logical core is a
safe default here.

What this cannot show is *why*. On this machine the plateau and the core count
both sit at 32, so the experiment cannot separate "the optimum tracks core count"
from "the optimum is ~32 on this storage and the core count coincides". Those
predict very different things on other hardware: a 4-core host with the same NVMe
would want ~4 workers under the first and ~32 under the second. Until a host with
a very different core-to-storage ratio is measured, `default_local_workers` is
justified empirically on this class of machine and not by mechanism.

### Reading from ZFS costs 45%

Phase 1 and phase 2 share a destination, a corpus, a protocol and a worker count,
differing only in the source filesystem — so they are a clean A/B on the read
side:

| Source (32 workers, same USB destination) | Median | Files/s | MB/s |
|---|---:|---:|---:|
| ext4 on the internal NVMe | 53.6 s | 24,609 | 210 |
| ZFS/lz4 on the Hynix | 77.9 s | 16,931 | 145 |

**ZFS is 45% slower to read 1.3M small files**, despite reading *fewer* physical
bytes: the corpus occupies 9.3 GB on ZFS after lz4 against 16 GB on ext4, where
1.3M small files pay 4K block slack. Per-file overhead — checksums, ARC
bookkeeping, decompression — dominates at this file size. Neither PCIe link is
close to saturated at 145-210 MB/s, so the difference is not the 3.0-vs-4.0 link.

### The slower direction is the one *reading* from USB

USB -> internal is 70.1 s against internal -> USB at 53.6 s, though the internal
drive is far faster on paper. Cold reads of 1.3M small files are latency-bound,
and USB SCSI/UAS translation adds per-I/O latency that parallelism only partly
hides. Both directions run at 161-210 MB/s against a 979 MB/s link, confirming
the workload stayed metadata-bound rather than hitting the USB ceiling.

---

## The core-count heuristic is wrong on small machines — orion (Pi 5), 2026-08-29

freya could not answer whether `default_local_workers` should track the logical
core count, because there the scaling plateau and the core count both sat at 32.
A 4-core host breaks that coincidence. The USB NVMe was physically moved from
freya to a Raspberry Pi 5, carrying the corpus with it.

**orion** — Raspberry Pi 5 Model B, 4 cores, 3 GB RAM, Gentoo, kernel 6.18.

| | Device | Link | Write | Read |
|---|---|---|---:|---:|
| `/` | SK hynix SHGP31-500GM-2 | PCIe 2.0 x1 | 434 MB/s | 471 MB/s |
| `/mnt/usb` | the same USB NVMe as freya | **5 Gbps** (was 10 on freya) | 357 MB/s | 322 MB/s |

16 GB of corpus against 3 GB of RAM means the source cannot be cached, so these
are honest by construction. Caches were still dropped before every rep.

### root NVMe -> USB NVMe, congress-1m (1,318,771 files), 2 reps

| Workers | Median | MAD | vs 1 | vs 4 (= core count) | Files/s | MB/s |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 458.3 s | 0.5% | 1.00x | 0.57x | 2,878 | 36 |
| 2 | 336.4 s | 0.4% | 1.36x | 0.77x | 3,921 | 49 |
| 4 | 260.0 s | 1.9% | 1.76x | 1.00x | 5,073 | 63 |
| 8 | 230.2 s | 3.6% | 1.99x | 1.13x | 5,729 | 71 |
| 16 | 218.3 s | 0.3% | 2.10x | 1.19x | 6,042 | 75 |
| 32 | 216.0 s | 0.9% | 2.12x | 1.20x | 6,107 | 76 |

### The finding

**Going past the core count is worth 20%.** Four workers on four cores — what
`available_parallelism()` produces today — is 1.20x slower than 16 or 32. The
curve flattens around 16; 32 buys a further 1%.

**The optimum did not move with the core count.** Put the two machines together:

| Host | Logical cores | Best worker count | Plateau |
|---|---:|---:|---|
| freya | 32 | 32 | flat to 64 |
| orion | 4 | 16-32 | flat past 16 |

An **8x** difference in core count produced no meaningful difference in the
optimum, which sat in the 16-32 range on both. That is the refutation freya could
not supply: the optimum tracks how many requests the storage will service
concurrently, not how many cores the host has. `available_parallelism()` matched
the optimum on freya by coincidence.

Two supporting observations from the Pi, both visible before the sweep finished:
scaling is already sub-linear *below* the core count (4 workers on 4 cores buys
1.76x, not 4x), and at the plateau the run uses **21% of the USB write ceiling**
with a load average near 2 on a 4-core box. The machine is waiting, not
computing, at every point on this curve.

### What this does not license

Raising the floor everywhere. `MACOS_WORKER_CAP = 4` exists because on macOS
additional workers measurably contend, so the same change that gains 20% here
could lose on another platform. The device also matters: 16 concurrent writers
suits NVMe and would likely thrash a spinning disk. The defensible reading is
that worker count should be driven by the storage, with core count as at best a
weak prior — see backlog V3.20.
