# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| mixed-content-churn-same-volume | same-volume | rsync-a | 0.1621 | 18.1% | 0.0473 | 5,357,568 | 0 | baseline | noisy (18%) |
| mixed-content-churn-same-volume | same-volume | xsync | 0.2292 | 14.0% | 0.0717 | 15,204,352 | 0 | 0.700 | noisy (18%) |
| mixed-content-churn-pipe | pipe | rsync-a | 0.1471 | 3.8% | 0.0543 | 5,652,480 | 0 | baseline | baseline |
| mixed-content-churn-pipe | pipe | rsync-az | 0.1523 | 3.2% | 0.0536 | 6,717,440 | 0 | 0.992 | yes |
| mixed-content-churn-pipe | pipe | xsync | 0.1230 | 10.0% | 0.0656 | 5,046,272 | 4,131 | 1.229 | yes |
| mixed-content-churn-pipe | pipe | xsync-raw | 0.1088 | 3.3% | 0.0637 | 4,816,896 | 20,633 | 1.356 | yes |
| ssh | ssh | *blocked* | - | - | - | - | - | - | the ssh route was not selected; native xsync-over-SSH and RsyncTransport rows require --routes ssh with --ssh-host |
