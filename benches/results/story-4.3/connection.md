# Story 4.3 — SSH connection-model benchmark

- schema: `xsync.connection-bench.v1`
- repetitions: 1
- setup kind: pipe-child (same transport line as production ssh)
- reference transfer: 201 files, 1867776 bytes, 180.08 ms
- per-session setup (streams=1): 3.78 ms

| streams | setup median (ms) | MAD (ms) | delta vs prev (ms) | transfer/setup |
|---:|---:|---:|---:|---:|
| 1 | 3.78 | 0.00 | 3.78 | 47.58 |
| 2 | 4.07 | 0.00 | 0.29 | 44.23 |
| 4 | 6.46 | 0.00 | 2.39 | 27.87 |
| 8 | 7.20 | 0.00 | 0.74 | 25.02 |
