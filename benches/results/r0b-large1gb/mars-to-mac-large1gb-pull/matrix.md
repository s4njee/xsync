# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 3 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| large1gb-initial-copy-ssh | ssh | rsync-a | 8.5627 | 1.0% | 4.1440 | 259,211,264 | 0 | baseline | baseline |
| large1gb-initial-copy-ssh | ssh | xsync | 8.4145 | 0.7% | 3.0680 | 149,618,688 | 887,995,366 | 1.016 | yes |
