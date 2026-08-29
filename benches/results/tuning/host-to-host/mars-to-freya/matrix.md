# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| congress-10k-initial-copy-ssh | ssh | rsync-a | 19.2705 | 1.5% | 0.5154 | 103,739,392 | 0 | baseline | baseline |
| congress-10k-initial-copy-ssh | ssh | xsync | 7.7353 | 1.6% | 0.9851 | 103,739,392 | 22,972,845 | 2.471 | yes |
