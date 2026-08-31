# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 3 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| large1gb-initial-copy-ssh | ssh | rsync-a | 8.6102 | 4.7% | 2.8862 | 25,100,288 | 0 | baseline | baseline |
| large1gb-initial-copy-ssh | ssh | xsync | 8.5048 | 6.3% | 2.9390 | 69,058,560 | 887,997,013 | 0.996 | yes |
