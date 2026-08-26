# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| congress-10k-initial-copy-same-volume | same-volume | rsync-a | 3.2671 | 2.9% | 5.2318 | 38,862,848 | 0 | baseline | baseline |
| congress-10k-initial-copy-same-volume | same-volume | xsync | 2.9926 | 0.3% | 10.0130 | 30,146,560 | 0 | 1.077 | yes |
| ssh | ssh | *blocked* | - | - | - | - | - | - | the ssh route was not selected; native xsync-over-SSH and RsyncTransport rows require --routes ssh with --ssh-host |
