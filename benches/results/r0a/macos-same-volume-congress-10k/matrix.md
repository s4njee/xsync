# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 3 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| congress-10k-initial-copy-same-volume | same-volume | rsync-a | 3.3425 | 0.5% | 4.9326 | 52,805,632 | 0 | baseline | baseline |
| congress-10k-initial-copy-same-volume | same-volume | xsync | 1.3603 | 2.4% | 2.2759 | 52,838,400 | 0 | 2.470 | yes |
| ssh | ssh | *blocked* | - | - | - | - | - | - | the ssh route was not selected; native xsync-over-SSH and RsyncTransport rows require --routes ssh with --ssh-host |
