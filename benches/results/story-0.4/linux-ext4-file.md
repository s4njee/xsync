# xsync local clone/reflink spike

- Schema: `xsync.clone-bench.report.v1`
- Host/filesystem: `mars` / `ext4`
- Object: `file` (1073741824 logical bytes)
- Source manifest: `3ee2e1df58cefbad106ae5f71fb64f0085a37e5df47cee785b767aa9b994e12b`
- Paranoid readback: `false`
- Platform clone available: `false`

## Summary

| Method | Median | MAD |
|---|---:|---:|
| BufferedVerified | 0.222019 s | 0.000315 s |
| CloneOrFallback | 0.223305 s | 0.002148 s |

Paired verified clone speedup: **0.993x** (MAD 0.017x).

Wall time covers the staged operation and publication. The independent oracle runs immediately afterward, outside the timer, and a sample is retained only when it passes.

## Repetitions

| Rep | Order | Method | Wall | Disposition | Cache | Verified |
|---:|---:|---|---:|---|---|---|
| 0 | 0 | CloneOrFallback | 0.219759 s | BufferedFallback | FirstPass | true |
| 0 | 1 | BufferedVerified | 0.222209 s | BufferedFallback | Warm | true |
| 1 | 0 | BufferedVerified | 0.220650 s | BufferedFallback | Warm | true |
| 1 | 1 | CloneOrFallback | 0.223305 s | BufferedFallback | Warm | true |
| 2 | 0 | CloneOrFallback | 0.219800 s | BufferedFallback | Warm | true |
| 2 | 1 | BufferedVerified | 0.222019 s | BufferedFallback | Warm | true |
| 3 | 0 | BufferedVerified | 0.219334 s | BufferedFallback | Warm | true |
| 3 | 1 | CloneOrFallback | 0.225453 s | BufferedFallback | Warm | true |
| 4 | 0 | CloneOrFallback | 0.223814 s | BufferedFallback | Warm | true |
| 4 | 1 | BufferedVerified | 0.222334 s | BufferedFallback | Warm | true |
