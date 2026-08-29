# congress-10k, mars <-> freya: the link-speed crossover

Date: 2026-08-24. Corpus `congress-10k` (11,280 files, 22,568 items, 96,542,108 logical
bytes, digest `fa7f75f7ef1ce81cb06af7492e51e58d5ae665769d9d5e92e33949f775b95e2e`),
five repetitions, rotated method order, independent manifest oracle on every run.

**The same corpus, the same two hosts, and the same two tools produce opposite verdicts
depending only on which way the data flows.**

| Direction | Link | `rsync -a` | `xsync` | Ratio | Comparable |
|---|---:|---:|---:|---:|---|
| mars -> freya (push) | 5.8 MB/s | 19.2705 s | **7.7353 s** | **2.471** | yes (1.5% / 1.6% MAD) |
| freya -> mars (pull) | 39.3 MB/s | **4.1029 s** | 4.7944 s | 0.840 | yes (2.8% / 4.6% MAD) |

xsync moved 22.97 MB and 22.92 MB over the wire respectively, against 96.54 MB logical —
a 4.2x reduction from adaptive zstd on this highly compressible JSON corpus. `rsync -a`
reports no wire accounting, and was run without `-z`.

## What this shows

This is the regime crossover that [TUNING.md](../../../TUNING.md) S7 and TUNING-TASKS T8
predicted and that no previous benchmark could show, because every earlier route was a LAN
or a local pipe where bandwidth was never the constraint.

- **When the link is the bottleneck, bytes on the wire decide.** At 5.8 MB/s, xsync's 4.2x
  compression converts directly into a 2.47x wall-clock win. rsync spends 19.3 s pushing
  96.5 MB because it is not compressing.
- **When the link is fast, per-file overhead decides.** At 39.3 MB/s the same 4.2x
  compression buys nothing, the transfer becomes metadata-bound, and xsync's per-file cost —
  the Epic T1 problem — puts it 1.19x behind.

The CPU columns corroborate the mechanism: in the fast direction both tools use ~1.0 s of
CPU and finish in ~4 s, so neither is CPU-bound; the difference is per-file work. In the
slow direction xsync spends 0.99 s of CPU to save 11.5 s of wall time.

## Environment, including two things that need attention

Both hosts are wired gigabit on the same subnet (`enp6s0` / `enp10s0`, both negotiating
1000 Mb/s, direct L2 route, no NIC errors, both idle at load < 0.2). Despite that:

- **The link is anomalously slow and strongly asymmetric.** Measured with 500 MB over SSH
  to `/dev/null`: mars -> freya **5.8 MB/s**, freya -> mars **39.3 MB/s**. That is 5% and
  31% of gigabit, with a 6.8x asymmetry between directions. freya runs k3s with flannel and
  ~50 veth interfaces, so an inbound netfilter/conntrack path is the first thing to check.
  This is worth diagnosing on its own merits; it is not an xsync problem.
- **The measurements are still valid**, because both tools ran over the identical link in
  the identical direction within a rotated schedule. But the *ratios* are properties of
  this link, not of gigabit Ethernet, and must not be quoted as though they were.

`freya`'s `/home` is ZFS with `compression=lz4`, so destination writes are compressed
beneath both tools equally. mars's destination is ext4 on NVMe.

## Defect found: native `RsyncTransport` fails on this corpus

The first push run failed outright:

```
xsync: rsync protocol error: receiver requested data for non-file index 6
flags 0xa000 (Directory bills/hr); remote rsync exited unsuccessfully (status 10)
```

`xsync --transport rsync` mishandles directory entries in the rsync file list on a deeply
nested real tree. Every previous `RsyncTransport` result came from synthetic corpora, which
never exposed it. Both runs here therefore carry `--skip-methods xsync-rsync-transport`,
recorded in each report's `skipped_methods` field. **This is a real bug and needs its own
story** — it is not a benchmark artifact.

## Harness changes made to run this

Three, all in `benches/scripts/release-bench.py`:

- `--pull-source` reverses the ssh route so the remote is the source and the destination is
  local, which is how the freya -> mars direction was measured without needing reverse SSH
  auth. It verifies locally against the same pinned manifest, valid only because the freya
  copy was first verified byte-identical to the mars source.
- `--skip-methods` excludes a named method and records the omission, so one broken method
  no longer costs the whole cell. The `rsync-a` baseline cannot be skipped.
- `build_id` now hashes in-process with `hashlib` instead of shelling out to `shasum`, which
  is macOS-only. The runner previously crashed at report time on Linux **after** completing
  every measurement.

## Caveats

- `--verify-sample 0.1` was used: repetition 1 is fully verified, later repetitions sample
  10% of content hashes. All oracles passed.
- Only `congress-10k` was run. At 5.8 MB/s, `congress-100k` would be roughly 2.5 minutes per
  transfer and `congress-1m` around 40 minutes, so neither is practical on this link until
  the asymmetry is diagnosed.
- The pull direction runs the client on mars and the server on freya, so client and server
  CPU roles do not swap between the two rows. Only the data direction reverses.
