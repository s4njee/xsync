# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 3 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| large1gb-initial-copy-ssh | ssh | rsync-a | 7.9186 | 0.3% | 3.0628 | 25,001,984 | 0 | baseline | baseline |
| large1gb-initial-copy-ssh | ssh | xsync | 8.2916 | 0.8% | 2.8122 | 135,397,376 | 887,997,013 | 0.958 | yes |
