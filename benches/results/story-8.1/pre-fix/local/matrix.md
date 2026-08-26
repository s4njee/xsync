# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| mixed-initial-copy-same-volume | same-volume | rsync-a | 0.1226 | 1.7% | 0.1186 | 5,439,488 | 0 | baseline | baseline |
| mixed-initial-copy-same-volume | same-volume | xsync | 0.2102 | 1.7% | 1.1198 | 5,242,880 | 0 | 0.582 | yes |
| mixed-no-op-second-sync-same-volume | same-volume | rsync-a | 0.0583 | 7.4% | 0.0140 | 5,095,424 | 0 | baseline | baseline |
| mixed-no-op-second-sync-same-volume | same-volume | xsync | 0.0278 | 3.5% | 0.0207 | 5,242,880 | 0 | 1.975 | yes |
| mixed-content-churn-same-volume | same-volume | *failed* | - | - | - | - | - | - | xsync correctness oracle failed (4 mismatches) |
| mixed-metadata-only-churn-same-volume | same-volume | *failed* | - | - | - | - | - | - | xsync correctness oracle failed (4 mismatches) |
| mixed-delete-same-volume | same-volume | rsync-a | 0.0386 | 2.6% | 0.0163 | 5,095,424 | 0 | baseline | baseline |
| mixed-delete-same-volume | same-volume | xsync | 0.0275 | 2.9% | 0.0213 | 5,341,184 | 0 | 1.479 | yes |
| deep-small-initial-copy-same-volume | same-volume | rsync-a | 0.2376 | 1.4% | 0.2292 | 5,242,880 | 0 | baseline | baseline |
| deep-small-initial-copy-same-volume | same-volume | xsync | 0.4702 | 4.3% | 2.4445 | 7,962,624 | 0 | 0.527 | yes |
| compressible-initial-copy-same-volume | same-volume | rsync-a | 0.0453 | 2.6% | 0.0237 | 5,226,496 | 0 | baseline | baseline |
| compressible-initial-copy-same-volume | same-volume | xsync | 0.0365 | 3.7% | 0.0863 | 4,210,688 | 0 | 1.255 | yes |
| incompressible-initial-copy-same-volume | same-volume | rsync-a | 0.0462 | 1.5% | 0.0233 | 5,210,112 | 0 | baseline | baseline |
| incompressible-initial-copy-same-volume | same-volume | xsync | 0.0329 | 6.8% | 0.0869 | 4,194,304 | 0 | 1.370 | yes |
| one-large-file-initial-copy-same-volume | same-volume | rsync-a | 0.0501 | 0.2% | 0.0252 | 5,341,184 | 0 | baseline | baseline |
| one-large-file-initial-copy-same-volume | same-volume | xsync | 0.0200 | 4.9% | 0.0092 | 3,850,240 | 0 | 2.599 | yes |
| mixed-initial-copy-cross-volume | cross-volume | rsync-a | 0.1291 | 1.3% | 0.1255 | 5,423,104 | 0 | baseline | baseline |
| mixed-initial-copy-cross-volume | cross-volume | xsync | 0.2360 | 1.4% | 1.2461 | 5,128,192 | 0 | 0.555 | yes |
| mixed-no-op-second-sync-cross-volume | cross-volume | rsync-a | 0.0378 | 2.1% | 0.0147 | 5,079,040 | 0 | baseline | baseline |
| mixed-no-op-second-sync-cross-volume | cross-volume | xsync | 0.0239 | 7.7% | 0.0200 | 5,160,960 | 0 | 1.676 | yes |
| mixed-content-churn-cross-volume | cross-volume | *failed* | - | - | - | - | - | - | xsync correctness oracle failed (4 mismatches) |
| mixed-metadata-only-churn-cross-volume | cross-volume | *failed* | - | - | - | - | - | - | xsync correctness oracle failed (4 mismatches) |
| mixed-delete-cross-volume | cross-volume | rsync-a | 0.0390 | 2.0% | 0.0163 | 5,079,040 | 0 | baseline | baseline |
| mixed-delete-cross-volume | cross-volume | xsync | 0.0274 | 1.9% | 0.0194 | 5,341,184 | 0 | 1.396 | yes |
| deep-small-initial-copy-cross-volume | cross-volume | rsync-a | 0.2519 | 2.0% | 0.2418 | 5,242,880 | 0 | baseline | baseline |
| deep-small-initial-copy-cross-volume | cross-volume | xsync | 0.4949 | 0.9% | 2.7406 | 7,929,856 | 0 | 0.517 | yes |
| compressible-initial-copy-cross-volume | cross-volume | rsync-a | 0.0450 | 0.9% | 0.0228 | 5,226,496 | 0 | baseline | baseline |
| compressible-initial-copy-cross-volume | cross-volume | xsync | 0.0331 | 0.3% | 0.0970 | 4,227,072 | 0 | 1.350 | yes |
| incompressible-initial-copy-cross-volume | cross-volume | rsync-a | 0.0458 | 0.6% | 0.0227 | 5,226,496 | 0 | baseline | baseline |
| incompressible-initial-copy-cross-volume | cross-volume | xsync | 0.0329 | 3.5% | 0.0940 | 4,079,616 | 0 | 1.393 | yes |
| one-large-file-initial-copy-cross-volume | cross-volume | rsync-a | 0.0549 | 0.6% | 0.0254 | 5,324,800 | 0 | baseline | baseline |
| one-large-file-initial-copy-cross-volume | cross-volume | xsync | 0.0324 | 1.4% | 0.0114 | 3,866,624 | 0 | 1.688 | yes |
| mixed-initial-copy-pipe | pipe | rsync-a | 0.1517 | 9.9% | 0.1254 | 5,406,720 | 0 | baseline | baseline |
| mixed-initial-copy-pipe | pipe | rsync-az | 0.1565 | 1.3% | 0.1269 | 8,339,456 | 0 | 0.985 | yes |
| mixed-initial-copy-pipe | pipe | xsync | 0.1660 | 0.6% | 0.1539 | 5,701,632 | 915,896 | 0.914 | yes |
| mixed-initial-copy-pipe | pipe | xsync-raw | 0.1636 | 1.8% | 0.1504 | 5,242,880 | 1,794,073 | 0.894 | yes |
| mixed-no-op-second-sync-pipe | pipe | rsync-a | 0.0742 | 2.8% | 0.0219 | 5,095,424 | 0 | baseline | baseline |
| mixed-no-op-second-sync-pipe | pipe | rsync-az | 0.0720 | 2.5% | 0.0221 | 5,095,424 | 0 | 1.011 | yes |
| mixed-no-op-second-sync-pipe | pipe | xsync | 0.0424 | 0.7% | 0.0288 | 4,653,056 | 0 | 1.695 | yes |
| mixed-no-op-second-sync-pipe | pipe | xsync-raw | 0.0413 | 2.0% | 0.0267 | 4,767,744 | 0 | 1.782 | yes |
| mixed-content-churn-pipe | pipe | rsync-a | 0.1012 | 5.8% | 0.0455 | 5,668,864 | 0 | baseline | baseline |
| mixed-content-churn-pipe | pipe | rsync-az | 0.0846 | 9.7% | 0.0458 | 6,766,592 | 0 | 1.027 | yes |
| mixed-content-churn-pipe | pipe | xsync | 0.0769 | 2.5% | 0.0579 | 5,046,272 | 4,131 | 1.108 | yes |
| mixed-content-churn-pipe | pipe | xsync-raw | 0.0777 | 2.1% | 0.0577 | 4,800,512 | 20,633 | 1.303 | yes |
| mixed-metadata-only-churn-pipe | pipe | rsync-a | 0.0761 | 3.9% | 0.0235 | 5,390,336 | 0 | baseline | baseline |
| mixed-metadata-only-churn-pipe | pipe | rsync-az | 0.0691 | 6.1% | 0.0239 | 5,472,256 | 0 | 1.068 | yes |
| mixed-metadata-only-churn-pipe | pipe | xsync | 0.0419 | 2.2% | 0.0299 | 4,915,200 | 4,128 | 1.805 | yes |
| mixed-metadata-only-churn-pipe | pipe | xsync-raw | 0.0428 | 3.4% | 0.0287 | 4,751,360 | 20,633 | 1.784 | yes |
| mixed-delete-pipe | pipe | rsync-a | 0.0750 | 3.9% | 0.0241 | 5,210,112 | 0 | baseline | baseline |
| mixed-delete-pipe | pipe | rsync-az | 0.0685 | 13.8% | 0.0236 | 5,226,496 | 0 | 0.963 | yes |
| mixed-delete-pipe | pipe | xsync | 0.0412 | 3.4% | 0.0267 | 4,669,440 | 0 | 1.678 | yes |
| mixed-delete-pipe | pipe | xsync-raw | 0.0409 | 4.0% | 0.0267 | 4,653,056 | 0 | 1.601 | yes |
| deep-small-initial-copy-pipe | pipe | rsync-a | 0.2643 | 6.7% | 0.2327 | 5,242,880 | 0 | baseline | baseline |
| deep-small-initial-copy-pipe | pipe | rsync-az | 0.2720 | 0.4% | 0.2350 | 6,422,528 | 0 | 0.958 | yes |
| deep-small-initial-copy-pipe | pipe | xsync | 0.3673 | 3.8% | 0.3423 | 6,520,832 | 93,928 | 0.697 | yes |
| deep-small-initial-copy-pipe | pipe | xsync-raw | 0.3662 | 2.7% | 0.3486 | 6,225,920 | 112,860 | 0.707 | yes |
| compressible-initial-copy-pipe | pipe | rsync-a | 0.0821 | 2.1% | 0.0295 | 5,226,496 | 0 | baseline | baseline |
| compressible-initial-copy-pipe | pipe | rsync-az | 0.0745 | 9.1% | 0.0269 | 8,388,608 | 0 | 1.102 | yes |
| compressible-initial-copy-pipe | pipe | xsync | 0.0438 | 2.5% | 0.0284 | 5,685,248 | 2,944 | 1.872 | yes |
| compressible-initial-copy-pipe | pipe | xsync-raw | 0.0431 | 1.1% | 0.0273 | 4,505,600 | 2,098,816 | 1.932 | yes |
| incompressible-initial-copy-pipe | pipe | rsync-a | 0.0807 | 2.6% | 0.0337 | 5,259,264 | 0 | baseline | baseline |
| incompressible-initial-copy-pipe | pipe | rsync-az | 0.0606 | 9.2% | 0.0341 | 8,568,832 | 0 | 1.094 | yes |
| incompressible-initial-copy-pipe | pipe | xsync | 0.0497 | 1.8% | 0.0310 | 5,701,632 | 2,098,816 | 1.639 | yes |
| incompressible-initial-copy-pipe | pipe | xsync-raw | 0.0446 | 6.3% | 0.0278 | 4,554,752 | 2,098,816 | 1.763 | yes |
| one-large-file-initial-copy-pipe | pipe | rsync-a | 0.0825 | 1.2% | 0.0323 | 5,324,800 | 0 | baseline | baseline |
| one-large-file-initial-copy-pipe | pipe | rsync-az | 0.0844 | 9.6% | 0.0366 | 8,880,128 | 0 | 0.982 | yes |
| one-large-file-initial-copy-pipe | pipe | xsync | 0.0535 | 1.0% | 0.0381 | 42,500,096 | 8,388,660 | 1.556 | yes |
| one-large-file-initial-copy-pipe | pipe | xsync-raw | 0.0531 | 1.2% | 0.0379 | 39,436,288 | 8,388,660 | 1.553 | yes |
| ssh | ssh | *blocked* | - | - | - | - | - | - | the ssh route was not selected; native xsync-over-SSH and RsyncTransport rows require --routes ssh with --ssh-host |
