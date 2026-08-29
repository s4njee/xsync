# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| congress-10k-initial-copy-ssh | ssh | rsync-a | 4.1029 | 2.8% | 1.0080 | 105,472,000 | 0 | baseline | baseline |
| congress-10k-initial-copy-ssh | ssh | xsync | 4.7944 | 4.6% | 1.0229 | 105,472,000 | 22,915,621 | 0.840 | yes |
