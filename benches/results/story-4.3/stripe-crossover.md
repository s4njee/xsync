# Story 4.3 — multi-stream striping crossover

- schema: `xsync.stripe-bench.v1`
- repetitions: 5
- many-small speedup at 4 streams: 0.90x
- pipe-child setup (optimistic lower bound; real ssh adds per-session RTT)

| corpus | logical bytes | streams | median (ms) | MAD (ms) | 4x/1x speedup |
|---:|---:|---:|---:|---:|---:|
| large-4M | 4194304 | 1 | 189.45 | 1.66 | 1.00x |
| large-4M | 4194304 | 4 | 193.58 | 1.61 | 0.98x |
| large-16M | 16777216 | 1 | 732.60 | 4.24 | 1.00x |
| large-16M | 16777216 | 4 | 519.23 | 0.66 | 1.41x |
| large-64M | 67108864 | 1 | 2922.67 | 35.67 | 1.00x |
| large-64M | 67108864 | 4 | 1504.37 | 20.91 | 1.94x |
| many-small | 1638400 | 1 | 254.54 | 2.82 | 1.00x |
| many-small | 1638400 | 4 | 283.12 | 7.60 | 0.90x |
