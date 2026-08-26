# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| congress-10k-initial-copy-same-volume | same-volume | rsync-a | 3.6201 | 0.8% | 5.7172 | 38,846,464 | 0 | baseline | baseline |
| congress-10k-initial-copy-same-volume | same-volume | xsync | 7.5636 | 2.4% | 29.5035 | 26,116,096 | 0 | 0.482 | yes |
| ssh | ssh | *blocked* | - | - | - | - | - | - | the ssh route was not selected; native xsync-over-SSH and RsyncTransport rows require --routes ssh with --ssh-host |
