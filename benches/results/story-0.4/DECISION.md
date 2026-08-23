# Story 0.4 decision — local clone/reflink and streaming I/O

## Decision

Select capability-gated APFS file cloning for later local-transfer integration. Select native
whole-tree APFS cloning only for a complete fresh destination tree with no exclusions, delete, or
merge semantics; Story 3.1 must use a safe native platform wrapper and retain ordinary-copy
fallback. On Linux, probe `FICLONE` and fall back transparently: the measured `mars.local` ext4
filesystem does not support it. Do not ship no-cache/read-ahead hints yet.

The spike is deliberately separate from xsync's transfer engine. Every candidate writes a sibling
stage and publishes the final name only after the operation succeeds. Capability errors and invalid
paranoid results remove the stage and use a physical buffered copy. `--paranoid` hashes the staged
and published file, or independently manifests the staged and published tree.

## Paired clone/reflink evidence

All xsync reports use five same-repetition pairs with alternating method order. Timed wall time
covers the staged operation and atomic publication. The independent manifest oracle runs directly
afterward, outside the timer, and failed outputs are never retained as samples. “First pass” makes
no cache-eviction claim.

| Host/filesystem | Object | Capability result | Ordinary median | Clone/fallback median | Paired speedup |
|---|---|---|---:|---:|---:|
| Apple M1 Max / APFS | 1 GiB file | clone succeeded 5/5 | 0.2880 s | 0.0030 s | 95.14x (MAD 0.96x) |
| Apple M1 Max / APFS | 10k-entry mixed tree | staged `cp -c -R` succeeded 5/5 | 1.4079 s | 1.5749 s | 0.917x (MAD 0.027x) |
| `mars.local` / ext4 | 1 GiB file | `FICLONE` unavailable; fallback 5/5 | 0.2220 s | 0.2233 s | 0.993x (MAD 0.017x) |

The APFS tree shell prototype is rejected because it walks the tree and is 8.3% slower. The
existing f2 native `clonefile(2)` root probe on the identical content-pinned tree measured a 147.4
ms warm median, 19.56x faster than sequential `copyfile`. That makes native whole-tree clone a
selected implementation candidate, but not permission to ship the slower shell route. The
workspace denies unsafe Rust, so integration needs a reviewed safe wrapper rather than local FFI.

Authoritative reports:

- `macos-apfs-file.json` / `.md`
- `macos-apfs-directory.json` / `.md`
- `linux-ext4-file.json` / `.md`
- `f2-macos-apfs-directory.txt` (qualified exploratory native-call evidence)

## Eligibility and fallback semantics

Whole-tree clone is eligible only when the computed target does not exist and the request has no
exclusions, `--delete`, or merge requirement. `source/ destination` targets `destination`;
`source destination` targets `destination/source-basename`. Existing file destinations remain in
place until a verified stage atomically replaces them. Tests also cover metadata, empty
directories, symlinks, subsequent source mutation (copy-on-write independence), a clone that fails
after writing a partial stage, a clone that falsely reports success with corrupt data, and paranoid
final-name readback.

On `mars.local`, `/home/sanjee` was rechecked as ext4 and `/tmp` as tmpfs. Every reflink attempt on
the ext4 corpus returned unsupported and the physical fallback produced the exact independent
manifest without a final partial file. This is the intended capability outcome, not a benchmark
failure.

## Cross-volume streaming hints

The f2 cross-volume APFS probe used the same 1 GiB content-pinned corpus and a separate temporary
APFS volume. A 32 MiB aligned `F_NOCACHE` buffer plus `F_RDAHEAD` measured 723.9 MiB/s, 2.37x the
grouped `copyfile` warm median. Its warmed 256 MiB sentinel had a 0.89x median paired post-copy
latency ratio, passing the 2.00x cache-pressure gate; ordinary `copyfile` also passed at 0.94x.

This is promising but not shippable evidence: f2 groups the throughput methods rather than
rotating paired order, and its streaming routine does not yet implement xsync's full metadata,
staging, verification, and cancellation contract. Therefore Story 0.4 selects no I/O hint for the
current product. A future integration may carry the 32 MiB macOS strategy only after a rotated
paired xsync run still improves and repeats the sentinel gate. No Linux hint is inferred from the
macOS result.

## Verification

- macOS: 75 routine workspace tests pass, the existing 100k stress test remains opt-in, and strict
  workspace Clippy passes.
- Linux (`mars.local`): 76 routine workspace tests pass, the same stress test remains opt-in, and
  strict workspace Clippy passes. Linux has one additional reversible non-UTF-8-path oracle test.
- The reports retain all repetitions, capability dispositions, cache labels, independent
  verification results, environment identity, logical bytes, and content manifest digest.
