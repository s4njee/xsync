# large1gb, one binary, four routes

Fills the gap R0a left: `large1gb` had no Mac→mars push cell, so there was no
fast-disk push control on the corpus the orion cells use. All four cells here
were run as a group on a single build so they are mutually comparable.

- **Binary:** `943b8ec49bc2-dirty` on all three endpoints, verified with
  `xs --version` on each. The uncommitted tree is identified by the digest of
  its diff, `bbe9b5f88ed215d8`; this run is therefore *identifiable* but **not
  reproducible from the repository** until that work is committed.
- **Corpus:** `large1gb`, 4 files, 847 MB, pinned digest `5588a3f1…`.
- Three interleaved repetitions after an unrecorded warmup, rotated method
  order, independent manifest oracle per run, `xsync-rsync-transport` skipped
  (it emits no core phase-boundary events).

| Cell | `rsync -a` | xsync | Paired ratio | MAD | R0a |
|---|---:|---:|---:|---:|---:|
| Mac → mars, ext4 | 8.61 s | 8.50 s | **0.996** | ≤6.3% | — |
| Mac → orion, ext4 | 7.93 s | 10.63 s | **0.753** | ≤1.0% | 0.760 |
| Mac → orion, tmpfs | 7.92 s | 8.29 s | **0.958** | ≤0.8% | 0.933 |
| mars → Mac, pull | 8.56 s | 8.41 s | **1.016** | ≤1.0% | 0.950 |

## What this settles

**The mars large-file deficit is corpus-specific, not general.** R0a measured
0.972 on `manga` (3.75 GiB, 56 files). On `large1gb` the same route is 0.996 —
parity. Whatever costs 2.8% on `manga` does not generalise to large files as a
class, and should not be treated as an open regression.

**The orion ext4 gap is stable and is the only remaining deficit.** 0.753 here
against 0.760 in R0a, on different binaries a day apart, with MAD ≤ 1%. Every
other cell is at or above parity.

**Removing the receiver's disk removes most of it**, consistently: 0.753 → 0.958
moving the destination to tmpfs, which is the same shape R0a measured
(0.760 → 0.933). This is the 4.66 diagnosis reproduced on the registered corpus.

**Pull crossed parity.** 0.950 → 1.016. Together with orion tmpfs (0.933 →
0.958) and orion ext4 unchanged (0.760 → 0.753), the pattern is that the
uncommitted work helped where the receiver's disk was *not* the constraint and
did nothing where it is — which is what 4.66 predicts.

## Caveat

`large1gb` is 4 files of ~210 MB. It exercises the chunked path but not
per-file overhead, and at ~8.5 s a run it is short enough that session setup is
a visible fraction. It is a control for the orion cells, not a general
large-file benchmark; `manga` remains the longer-running case.
