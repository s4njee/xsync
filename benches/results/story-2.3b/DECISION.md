# Story 2.3b decision — strategy calibration and shared scheduling

Date: 2026-08-23

## Decision

- Keep the logical defaults at a **32 MiB small-file batch target** and **16 MiB large-file
  segment**. The batch target reduces scheduling events materially from 8 MiB while avoiding the
  larger logical reservation of 64 MiB. The 16 MiB segment is the fastest of the tested segment
  sizes for the large-file dispatch workload and keeps per-stream queue reservations bounded.
- Use a shared bounded local queue for small and medium work. Local worker count is no longer used
  to assign those items, so a slow worker cannot head-of-line block idle workers.
- Keep large-file ranges on stable per-stream queues. Ownership is derived from logical chunk
  order and is independent of local worker count; ranges are disjoint and no two stream queues
  receive the same range.
- Logical batches and segments are strategy inputs, not wire-frame inputs. A transport may split one
  logical item across several frames without changing membership or stable stream ownership.

## Matrix

`strategy-matrix.json` contains 180 cells: three synthetic corpus shapes (`flat-small`,
`deep-small`, and `large-file`), four batch targets (8/16/32/64 MiB), three segment sizes
(4/8/16 MiB), and five worker counts (1/2/4/8/16). Every cell has five repetitions. The runner
uses a shared local queue with capacity two and one stable stream queue per configured worker count.

The matrix measures metadata dispatch and scheduling overhead, not filesystem or network transfer
throughput. On this host, representative aggregate medians were:

| Sweep | Candidate | Median dispatch | Interpretation |
|---|---:|---:|---|
| Flat small batch | 32 MiB | 645,333 ns | 17.8% below 8 MiB; 1.4% above 64 MiB |
| Deep small batch | 32 MiB | 767,625 ns | 15.1% below 8 MiB; 2.5% below 64 MiB |
| Large-file segment | 16 MiB | 813,667 ns | 2.3x below 8 MiB; 2.6x below 4 MiB |

The small-file rows all classify 50,000 4 KiB entries as batches; the large-file rows classify one
10 GiB entry into 2,560, 1,280, or 640 chunks at 4, 8, or 16 MiB respectively. Worker-count rows
cover 1/2/4/8/16 but do not select a local I/O default; that choice remains a host/filesystem
decision outside metadata dispatch.

## Bounds and correctness

At queue capacity two and 16 streams, the default logical queue reservation is **576 MiB**:

`2 * max(32 MiB batch, 32 MiB whole file) + 16 * 2 * 16 MiB chunks`.

The strategy queues carry metadata-only work items. Workers read file payloads after dequeue, so
576 MiB is a conservative logical reservation, not an allocation made by `xsync-core`; the
transport must enforce its own frame and in-flight byte limits.

The shared scheduler tests include a deliberately sleeping local worker and verify that a second
worker drains the shared queue. Stable-stream tests verify deterministic stream ownership and
disjoint ranges. The logical strategy configuration contains no wire-frame size, so changing a
transport frame limit cannot change logical batch membership.

## Reproduction

```text
cargo run --release -p xsync-engine-bench --bin xsync-strategy-bench
```

The command writes the checked-in JSON and Markdown artifacts. It requires at least five
repetitions for an acceptance run.

## Artifacts

- `strategy-matrix.json`: machine-readable five-repetition matrix.
- `strategy-matrix.md`: same matrix rendered as Markdown.
- `crates/xsync-core/src/strategy.rs`: shared scheduler, configurable logical thresholds, and
  queue-bound calculation.
