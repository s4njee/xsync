# T1 — Gate cloud-placeholder detection on the policy that uses it

**Change under test:** `crates/xsync-core/src/local.rs` now runs
`cloud::is_placeholder` only when `options.cloud_files` is `Skip` or `Error`.
Under the default `Download` policy every entry is retained whatever the answer
is, so the inspection was pure cost.

`cloud::is_placeholder` is implemented as a spawn of
`/usr/bin/xattr -p com.apple.fileprovider.fpfs#P <path>` — the core crate denies
`unsafe_code` and has no safe wrapper for the underlying query — so it costs one
`fork` + `exec` per regular file. On `congress-10k` that is **11,280 process
creations during planning, before any byte moves.**

## Result

Measured 2026-08-26 on the M1 Max, `congress-10k` (11,280 files, 96,542,108
logical bytes, 22,568 items), local same-volume APFS, initial copy, five
repetitions per method, rotated method order, independent manifest oracle after
every run.

| | before | after | change |
|---|---:|---:|---:|
| xsync median wall | 39.609 s | **8.217 s** | **4.8x faster** |
| xsync median CPU | 29.388 s | **7.013 s** | **4.2x less** |
| xsync MAD/median | 16.5% | **4.7%** | inside the 15% policy |
| paired ratio vs `rsync -a` | 0.128 | **0.867** | 6.8x better |
| peak RSS | 29,687,808 | 36,945,920 | +24% |
| oracle | 10/10 pass | 10/10 pass | unchanged |

CPU delta is 22.375 s over 11,280 files — **≈1.98 ms of CPU per file**, which is
the right order of magnitude for `fork` + `exec` + `dyld` on macOS.

## Why this is directional evidence, not gate-able evidence

**The host was heavily contended for both runs.** Load average was 265 (a `zstd`
process at 485% CPU and a UTM/QEMU VM at 178%), and neither run had an idle
machine. Consequences:

- The **before** run is `noisy` under the Epic 0 policy — 26.8% MAD/median on the
  `rsync -a` baseline and 16.5% on xsync — so it is reported but is **not**
  gate-able.
- The **after** run is internally comparable (9.3% baseline, 4.7% xsync, both
  inside 15%) and the harness labels it `Comparable: yes`, but a before/after
  *pair* is only as good as its worse half.

The result is still worth recording, for one reason: **the environment moved
against the change, not with it.** The same-run `rsync -a` baseline got *slower*
between the two runs, 5.294 s → 7.336 s, so the machine was more loaded during
the "after" measurement. An environmental explanation for a 4.2x CPU reduction
would have to run the wrong way.

**A five-repetition rerun on an idle host is still required** before this number
is quoted anywhere as a T1 result.

Do not cross-compare these rows against `../hash-cached/` or against TUNING.md §3:
those were measured on an idle machine and at a revision that predates this code
entirely (see below).

## No existing T1 baseline contains this cost

`crates/xsync-core/src/cloud.rs` and the `is_placeholder` call in `local.rs` were
both introduced in **`8ca26cce`**. Every checked-in T1 report is stamped
`f5e10179`, which is four commits earlier:

```
f5e10179  story 4.4: define rsync wire compatibility contract   <- T1 reports
cc6c68a4  Align Windows cfg signatures and land post-4.4 engine work.
633ac004  deployment: add cross-platform CI and Windows launcher
a74ad05e  release: prepare v0.1
8ca26cce  release: prepare v0.1.1                               <- cloud.rs added
```

So `buffer-sized/`, `hash-baseline/`, `hash-cached/` and `clone-threshold/` all
predate placeholder detection, and TUNING.md §3's `1.9 ms of kernel time per
file` is **not** this spawn — the resemblance to the 1.98 ms/file measured here is
a coincidence and must not be reported as attribution. It also means Story T1.3's
recorded `0.515` paired wall ratio was measured on a binary without this cost, so
the true pre-change state of `main` was worse than T1.3 records.

## Reproducing

```bash
cargo build --release -p xsync -p xsync-bench
python3 benches/scripts/release-bench.py \
  --corpus congress-10k --workload initial-copy \
  --routes same-volume --repetitions 5 \
  --xsync target/release/xs --bench target/release/xsync-bench \
  --out /tmp/xsync-cloud-gate
```

The `before` side is the same command with an `xs` built from `c142c677`.

## Artifacts

`input-*.json` (every repetition, unaggregated), `report-*.json`, `report-*.md`,
`schedule-*.json`, `matrix.json` and `matrix.md` for each side.

**The 22 per-repetition oracle manifests are deliberately not checked in.** They
are 9.3 MB each — 204 MB for the pair — and `benches/results/tuning/` is already
715 MB of committed evidence. Each manifest's verdict is recorded in the
corresponding report as the per-repetition `oracle` column, and all 20 transfer
runs plus their verifications passed. This departs from the precedent set by the
sibling directories; if the raw manifests are wanted, rerun the command above,
which regenerates them.
