# xsync local clone/reflink spike

- Schema: `xsync.clone-bench.report.v1`
- Host/filesystem: `MacBookPro` / `apfs`
- Object: `file` (1073741824 logical bytes)
- Source manifest: `3ee2e1df58cefbad106ae5f71fb64f0085a37e5df47cee785b767aa9b994e12b`
- Paranoid readback: `false`
- Platform clone available: `true`

## Summary

| Method | Median | MAD |
|---|---:|---:|
| BufferedVerified | 0.288000 s | 0.014880 s |
| CloneOrFallback | 0.003016 s | 0.000185 s |

Paired verified clone speedup: **95.138x** (MAD 0.957x).

Wall time covers the staged operation and publication. The independent oracle runs immediately afterward, outside the timer, and a sample is retained only when it passes.

## Repetitions

| Rep | Order | Method | Wall | Disposition | Cache | Verified |
|---:|---:|---|---:|---|---|---|
| 0 | 0 | CloneOrFallback | 0.002834 s | Cloned | FirstPass | true |
| 0 | 1 | BufferedVerified | 0.302879 s | BufferedFallback | Warm | true |
| 1 | 0 | BufferedVerified | 0.310131 s | BufferedFallback | Warm | true |
| 1 | 1 | CloneOrFallback | 0.003227 s | Cloned | Warm | true |
| 2 | 0 | CloneOrFallback | 0.003016 s | Cloned | Warm | true |
| 2 | 1 | BufferedVerified | 0.286953 s | BufferedFallback | Warm | true |
| 3 | 0 | BufferedVerified | 0.288000 s | BufferedFallback | Warm | true |
| 3 | 1 | CloneOrFallback | 0.003357 s | Cloned | Warm | true |
| 4 | 0 | CloneOrFallback | 0.002831 s | Cloned | Warm | true |
| 4 | 1 | BufferedVerified | 0.268214 s | BufferedFallback | Warm | true |
