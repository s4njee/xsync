# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 3 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| large1gb-initial-copy-ssh | ssh | rsync-a | 7.9251 | 0.4% | 3.1993 | 25,133,056 | 0 | baseline | baseline |
| large1gb-initial-copy-ssh | ssh | xsync | 10.6318 | 1.0% | 2.8998 | 76,218,368 | 887,997,013 | 0.753 | yes |
