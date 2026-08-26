# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| deep-small-initial-copy-ssh | ssh | rsync-a | 0.2993 | 5.7% | 0.0720 | 7,782,400 | 0 | baseline | baseline |
| deep-small-initial-copy-ssh | ssh | xsync | 0.3429 | 2.1% | 0.1211 | 7,864,320 | 93,060 | 0.873 | yes |
| deep-small-initial-copy-ssh | ssh | xsync-rsync-transport | 0.4568 | 2.1% | 0.1320 | 7,815,168 | 303,195 | 0.642 | yes |
| flat-small-no-op-second-sync-ssh | ssh | rsync-a | 0.2334 | 9.1% | 0.0383 | 7,208,960 | 0 | baseline | baseline |
| flat-small-no-op-second-sync-ssh | ssh | xsync | 0.2275 | 4.0% | 0.0559 | 7,520,256 | 0 | 1.032 | yes |
| flat-small-no-op-second-sync-ssh | ssh | xsync-rsync-transport | 0.3802 | 5.6% | 0.0661 | 7,274,496 | 60,084 | 0.614 | yes |
| compressible-initial-copy-ssh | ssh | rsync-a | 0.7957 | 0.2% | 0.0647 | 23,625,728 | 0 | baseline | baseline |
| compressible-initial-copy-ssh | ssh | xsync | 0.7277 | 21.8% | 0.0394 | 7,585,792 | 2,944 | 1.096 | noisy (22%) |
| compressible-initial-copy-ssh | ssh | xsync-rsync-transport | 0.9642 | 50.1% | 0.0760 | 22,347,776 | 2,102,303 | 0.609 | noisy (50%) |
| incompressible-initial-copy-ssh | ssh | rsync-a | 0.4747 | 42.6% | 0.0549 | 21,921,792 | 0 | baseline | noisy (43%) |
| incompressible-initial-copy-ssh | ssh | xsync | 0.5958 | 25.9% | 0.0614 | 23,396,352 | 2,098,816 | 1.089 | noisy (43%) |
| incompressible-initial-copy-ssh | ssh | xsync-rsync-transport | 0.9118 | 8.0% | 0.0774 | 24,395,776 | 2,102,303 | 0.645 | noisy (43%) |
| one-large-file-initial-copy-ssh | ssh | rsync-a | 0.8820 | 6.7% | 0.0854 | 128,122,880 | 0 | baseline | baseline |
| one-large-file-initial-copy-ssh | ssh | xsync | 0.8783 | 6.4% | 0.0791 | 100,483,072 | 8,388,660 | 1.030 | yes |
| one-large-file-initial-copy-ssh | ssh | xsync-rsync-transport | 1.0357 | 36.6% | 0.1071 | 92,372,992 | 8,391,901 | 0.645 | noisy (37%) |
