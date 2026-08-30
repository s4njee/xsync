# `--streams` across platforms

What parallel SSH streams actually do, measured on four hosts and three corpora.

**Summary in one line:** streams only ever help large files, by construction, and
the flag has been a net loss on every small-file corpus measured — which is why
the stream count should be chosen from the corpus rather than typed by the user.

## Why it can only help large files

`sync_push_server_streams` partitions the plan on `MAX_DATA_SEGMENT` (8 MB):

- files **at or below** it are written by the **control session** — one connection
- files **above** it are striped as byte ranges across the **data threads**

So the data threads, which are the entire point of the flag, are idle on a corpus
of small files. Every measurement below follows from that.

| Corpus | Bytes in files >8 MB | Streams help? |
|---|---:|---|
| congress-100k | 0% | no — pure cost |
| cb7 | 68% | not measured as a streams question |
| Manga | 99% | yes, modestly |

## Three bugs, found in sequence

The flag was broken in three independent ways. Each was hidden by the previous
one, and each needed a different corpus or platform to expose.

### 1. The small-file sender was a stale copy — 12.2x

`--streams 8` on congress-100k (mac → mars) took **211.5 s** against **17.3 s**
for a single stream. The multi-stream path carried its own copy of the small-file
sender that never received the batching work: every file cost two synchronous
round trips.

The original diagnosis in `BENCHMARKv2.md` blamed contention across streams. That
was wrong, and the disproof was that **cost did not vary with stream count** —
2, 4 and 8 streams all landed within 5% of each other, which contention cannot
explain. Fixed by extracting `send_small_files_batched` and calling it from both
paths. Measured on cb7: **149.3 s → 55.3 s, 2.70x**.

### 2. Metadata was still one round trip per entry — a further 7.7x

The fix above covered small *files* and not *metadata*. Three loops still took a
synchronous ack per entry: directory creation, symlink creation, and the final
`SetDirectory` pass.

congress gives every bill its own directory — **1,079 directories for 1,076
files** — so this was very nearly one round trip per file. To a Raspberry Pi at
`--streams 2`:

| Stage | congress-1k |
|---|---:|
| before | 18.28 s (59 files/s) |
| after directories + symlinks | 9.33 s (115 files/s) |
| after the metadata pass | **2.37 s (455 files/s)** |

Manga never showed this: seven files and almost no directories. It took a corpus
with roughly one directory per file to make it visible.

### 3. It did not work against Windows at all

Every multi-stream transfer to a Windows host died with `server stream
disconnected`. Only the single-stream path probes the remote shell family — it
tries POSIX, retries as Windows, and caches the answer. A process invoked
straight with `--streams N` skipped that discovery, and cmd.exe could not parse
the POSIX-quoted command.

## Where streams pay: large files

mac → mars, Manga, two tiers. This is the case the flag was designed for.

| Streams | One 885 MB file | 1,383 MB / 7 files |
|---|---:|---:|
| 1 | 12.2 s | 16.8 s |
| 2 | 10.1 s | 14.9 s |
| 4 | **9.9 s** | **14.4 s** |
| 8 | 10.8 s | 16.8 s |

**~1.2x, peaking at 4 streams**, then declining. With connection setup subtracted,
4 streams transferred at 106 MB/s against a measured 106 MB/s wire ceiling — the
flag doing exactly what it should, and stopping when the network runs out.

Windows agrees on large files:

| Config | Manga 1,383 MB → Windows |
|---|---:|
| default | 20.23 s (68 MB/s) |
| `--streams 4` | **19.43 s (71 MB/s)** |
| default (bracket close) | 20.61 s |

Marginally faster, bracket holding. Windows is not hostile to streams; it is
hostile to streams *on small files*.

## Where streams cost: small files

congress-100k, after all three fixes.

| Target | default | `--streams 2` | `--streams 4` | `--streams 8` |
|---|---:|---:|---:|---:|
| Windows (7900X) | **99.8 s** | 143.5 s | 152.4 s | 149.8 s |
| Pi 5 (orion) | 18.1 s | 26.4 s | 35.3 s | 31.6 s |

Windows pays **1.44-1.53x** for the flag. The bracket held there (99.77 s open,
98.51 s close), so those figures are sound.

> **The Pi figures are not internally comparable.** Its bracket drifted 1.5x
> across the session — opening default 18.13 s, closing 27.11 s — most likely
> thermal, since a Pi 5 throttles under sustained load. What survives is the
> *magnitude* of the fix, which dwarfs the drift: the same sweep measured
> **343 s** at `--streams 2` before the metadata fix and **26 s** after. The fine
> ranking between 2, 4 and 8 does not.

## Pull direction

Streams are neutral when pulling, where the client writes locally:

| Config | Mac ← Windows, congress-100k |
|---|---:|
| default | 28.40 s |
| `--streams 4` | 27.88 s |
| default (close) | 28.74 s |

## What still costs, and is not this flag's fault

- **The control session negotiates `capabilities=0x0`** — no compression. Every
  small file crosses it uncompressed, and congress is ~15x compressible. This is
  why `--streams 2` on congress-1k is still 2.37 s against a single stream's
  0.60 s after the metadata fix.
- **Connections are established in a sequential loop**, ~1.3 s at 4-8 streams.
- **`--streams 16` fails** with `Broken pipe`: N+1 SSH connections exceed
  OpenSSH's default `MaxStartups` of 10.

All three are tracked as V3.18.

## The conclusion: choose the count from the corpus

xsync knows the size distribution after planning. The stream count should be
derived from the share of transferred **bytes** in files above
`MAX_DATA_SEGMENT` — not from file count, since cb7 is 82.6% small files by count
but 68% large by bytes, and a count-based rule would disable the flag on the
corpus most likely to benefit.

An explicit `--streams N` must still win, the per-connection cost has to be
covered by the expected gain, and Windows wants a lower cap than Linux. Specified
as backlog item **4.14**.
