# Story 4.3 — SSH connection-model benchmark

- schema: `xsync.connection-bench.v1`
- repetitions: 5
- setup kind: pipe-child (same transport line as production ssh)
- reference transfer: 201 files, 1867776 bytes, 22.01 ms
- per-session setup (streams=1): 3.03 ms

| streams | setup median (ms) | MAD (ms) | delta vs prev (ms) | transfer/setup |
|---:|---:|---:|---:|---:|
| 1 | 3.03 | 0.30 | 3.03 | 7.27 |
| 2 | 3.50 | 0.22 | 0.48 | 6.28 |
| 4 | 5.63 | 0.61 | 2.12 | 3.91 |
| 8 | 8.99 | 0.69 | 3.36 | 2.45 |
