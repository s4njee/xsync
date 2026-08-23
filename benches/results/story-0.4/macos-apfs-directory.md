# xsync local clone/reflink spike

- Schema: `xsync.clone-bench.report.v1`
- Host/filesystem: `MacBookPro` / `apfs`
- Object: `directory` (35525769 logical bytes)
- Source manifest: `43b556d9f63347c185b822da604257d090fac4ddb73318df17f5c6b05382abc6`
- Paranoid readback: `false`
- Platform clone available: `true`

## Summary

| Method | Median | MAD |
|---|---:|---:|
| BufferedVerified | 1.407944 s | 0.023931 s |
| CloneOrFallback | 1.574900 s | 0.048212 s |

Paired verified clone speedup: **0.917x** (MAD 0.027x).

Wall time covers the staged operation and publication. The independent oracle runs immediately afterward, outside the timer, and a sample is retained only when it passes.

## Repetitions

| Rep | Order | Method | Wall | Disposition | Cache | Verified |
|---:|---:|---|---:|---|---|---|
| 0 | 0 | CloneOrFallback | 1.526688 s | Cloned | FirstPass | true |
| 0 | 1 | BufferedVerified | 1.407944 s | BufferedFallback | Warm | true |
| 1 | 0 | BufferedVerified | 1.443815 s | BufferedFallback | Warm | true |
| 1 | 1 | CloneOrFallback | 1.574900 s | Cloned | Warm | true |
| 2 | 0 | CloneOrFallback | 1.630509 s | Cloned | Warm | true |
| 2 | 1 | BufferedVerified | 1.384013 s | BufferedFallback | Warm | true |
| 3 | 0 | BufferedVerified | 1.390046 s | BufferedFallback | Warm | true |
| 3 | 1 | CloneOrFallback | 1.661754 s | Cloned | Warm | true |
| 4 | 0 | CloneOrFallback | 1.547918 s | Cloned | Warm | true |
| 4 | 1 | BufferedVerified | 1.461317 s | BufferedFallback | Warm | true |
