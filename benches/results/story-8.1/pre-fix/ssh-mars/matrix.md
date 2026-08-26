# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| deep-small-initial-copy-ssh | ssh | rsync-a | 0.7943 | 2.9% | 0.0673 | 7,487,488 | 0 | baseline | baseline |
| deep-small-initial-copy-ssh | ssh | xsync | 8.7309 | 0.9% | 0.6915 | 7,028,736 | 93,928 | 0.091 | yes |
| deep-small-initial-copy-ssh | ssh | xsync-rsync-transport | 0.9410 | 25.7% | 0.1126 | 7,733,248 | 303,195 | 0.869 | noisy (26%) |
| flat-small-no-op-second-sync-ssh | ssh | rsync-a | 0.6570 | 64.5% | 0.0351 | 7,192,576 | 0 | baseline | noisy (65%) |
| flat-small-no-op-second-sync-ssh | ssh | xsync | 0.2362 | 6.3% | 0.0453 | 7,897,088 | 0 | 2.970 | noisy (65%) |
| flat-small-no-op-second-sync-ssh | ssh | xsync-rsync-transport | 1.1216 | 67.2% | 0.0597 | 7,241,728 | 60,084 | 0.580 | noisy (67%) |
| compressible-initial-copy-ssh | ssh | rsync-a | 0.3043 | 6.4% | 0.0681 | 22,855,680 | 0 | baseline | baseline |
| compressible-initial-copy-ssh | ssh | xsync | 1.0408 | 0.9% | 0.0761 | 7,110,656 | 2,944 | 0.364 | yes |
| compressible-initial-copy-ssh | ssh | xsync-rsync-transport | 0.9552 | 14.8% | 0.0957 | 22,429,696 | 2,102,303 | 0.307 | yes |
| incompressible-initial-copy-ssh | ssh | rsync-a | 0.3554 | 29.3% | 0.0518 | 22,872,064 | 0 | baseline | noisy (29%) |
| incompressible-initial-copy-ssh | ssh | xsync | 1.0474 | 46.9% | 0.0508 | 7,323,648 | 2,098,816 | 0.615 | noisy (47%) |
| incompressible-initial-copy-ssh | ssh | xsync-rsync-transport | 1.2235 | 41.1% | 0.0624 | 22,265,856 | 2,102,303 | 0.622 | noisy (41%) |
| one-large-file-initial-copy-ssh | ssh | rsync-a | 1.0199 | 5.7% | 0.0917 | 100,827,136 | 0 | baseline | baseline |
| one-large-file-initial-copy-ssh | ssh | xsync | 1.0207 | 14.1% | 0.0870 | 73,121,792 | 8,388,660 | 0.876 | yes |
| one-large-file-initial-copy-ssh | ssh | xsync-rsync-transport | 1.7569 | 19.8% | 0.1051 | 77,119,488 | 8,391,901 | 0.586 | noisy (20%) |
