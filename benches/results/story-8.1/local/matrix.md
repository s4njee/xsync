# xsync Story 8.1 release benchmark matrix

Tier `smoke`, seed `42`, 5 repetitions per method, rotated method order, independent manifest oracle per run.

`ratio` is the same-repetition paired speedup of xsync against `rsync -a` (above 1.0 means xsync is faster). Per the Epic 0 policy a row is comparable only when both it and its baseline hold MAD/median at or below 15%; rows marked `noisy` are reported but are **not** gate-able evidence.

| Cell | Route | Method | Median wall s | MAD/median | Median CPU s | Peak RSS | Median wire B | Ratio vs rsync -a | Comparable |
|---|---|---|---:|---:|---:|---:|---:|---:|---|
| mixed-initial-copy-same-volume | same-volume | rsync-a | 0.1406 | 1.7% | 0.1149 | 5,390,336 | 0 | baseline | baseline |
| mixed-initial-copy-same-volume | same-volume | xsync | 0.2406 | 13.3% | 1.1234 | 5,062,656 | 0 | 0.534 | yes |
| mixed-no-op-second-sync-same-volume | same-volume | rsync-a | 0.0591 | 9.8% | 0.0132 | 5,095,424 | 0 | baseline | baseline |
| mixed-no-op-second-sync-same-volume | same-volume | xsync | 0.0299 | 4.9% | 0.0188 | 5,111,808 | 0 | 1.980 | yes |
| mixed-content-churn-same-volume | same-volume | rsync-a | 0.0645 | 0.8% | 0.0338 | 5,324,800 | 0 | baseline | baseline |
| mixed-content-churn-same-volume | same-volume | xsync | 4.0888 | 5.4% | 0.4140 | 15,548,416 | 0 | 0.016 | yes |
| mixed-metadata-only-churn-same-volume | same-volume | rsync-a | 0.0496 | 26.8% | 0.0159 | 5,111,808 | 0 | baseline | noisy (27%) |
| mixed-metadata-only-churn-same-volume | same-volume | xsync | 0.0376 | 1.8% | 0.0327 | 5,292,032 | 0 | 1.064 | noisy (27%) |
| mixed-delete-same-volume | same-volume | rsync-a | 0.0396 | 0.2% | 0.0162 | 5,095,424 | 0 | baseline | baseline |
| mixed-delete-same-volume | same-volume | xsync | 0.0349 | 8.9% | 0.0208 | 5,292,032 | 0 | 1.107 | yes |
| mixed-type-replacement-same-volume | same-volume | rsync-a | 0.0369 | 1.5% | 0.0152 | 5,095,424 | 0 | baseline | baseline |
| mixed-type-replacement-same-volume | same-volume | xsync | 0.0307 | 8.6% | 0.0197 | 5,308,416 | 0 | 1.297 | yes |
| mixed-interrupted-resume-same-volume | same-volume | rsync-a | 0.0808 | 0.2% | 0.0699 | 5,373,952 | 0 | baseline | baseline |
| mixed-interrupted-resume-same-volume | same-volume | xsync | 0.1188 | 2.9% | 0.5827 | 5,505,024 | 0 | 0.680 | yes |
| deep-small-initial-copy-same-volume | same-volume | rsync-a | 0.2639 | 0.8% | 0.2316 | 5,210,112 | 0 | baseline | baseline |
| deep-small-initial-copy-same-volume | same-volume | xsync | 0.4531 | 2.1% | 2.4601 | 8,110,080 | 0 | 0.578 | yes |
| compressible-initial-copy-same-volume | same-volume | rsync-a | 0.0464 | 6.0% | 0.0214 | 5,226,496 | 0 | baseline | baseline |
| compressible-initial-copy-same-volume | same-volume | xsync | 0.0339 | 2.6% | 0.0962 | 4,128,768 | 0 | 1.493 | yes |
| incompressible-initial-copy-same-volume | same-volume | rsync-a | 0.0434 | 3.2% | 0.0202 | 5,210,112 | 0 | baseline | baseline |
| incompressible-initial-copy-same-volume | same-volume | xsync | 0.0323 | 0.5% | 0.0993 | 4,177,920 | 0 | 1.396 | yes |
| one-large-file-initial-copy-same-volume | same-volume | rsync-a | 0.0497 | 7.4% | 0.0228 | 5,324,800 | 0 | baseline | baseline |
| one-large-file-initial-copy-same-volume | same-volume | xsync | 0.0198 | 1.2% | 0.0090 | 3,883,008 | 0 | 2.541 | yes |
| mixed-initial-copy-cross-volume | cross-volume | rsync-a | 0.1220 | 1.5% | 0.1145 | 5,406,720 | 0 | baseline | baseline |
| mixed-initial-copy-cross-volume | cross-volume | xsync | 0.2159 | 1.4% | 1.2185 | 5,111,808 | 0 | 0.573 | yes |
| mixed-no-op-second-sync-cross-volume | cross-volume | rsync-a | 0.0370 | 1.6% | 0.0142 | 5,079,040 | 0 | baseline | baseline |
| mixed-no-op-second-sync-cross-volume | cross-volume | xsync | 0.0225 | 4.6% | 0.0186 | 5,357,568 | 0 | 1.643 | yes |
| mixed-content-churn-cross-volume | cross-volume | rsync-a | 0.1198 | 3.5% | 0.0448 | 5,341,184 | 0 | baseline | baseline |
| mixed-content-churn-cross-volume | cross-volume | xsync | 3.7814 | 4.2% | 0.3128 | 15,450,112 | 0 | 0.032 | yes |
| mixed-metadata-only-churn-cross-volume | cross-volume | rsync-a | 0.0604 | 4.2% | 0.0162 | 5,144,576 | 0 | baseline | baseline |
| mixed-metadata-only-churn-cross-volume | cross-volume | xsync | 0.0306 | 7.4% | 0.0343 | 5,423,104 | 0 | 1.986 | yes |
| mixed-delete-cross-volume | cross-volume | rsync-a | 0.0383 | 0.9% | 0.0167 | 5,095,424 | 0 | baseline | baseline |
| mixed-delete-cross-volume | cross-volume | xsync | 0.0259 | 3.9% | 0.0206 | 5,259,264 | 0 | 1.627 | yes |
| mixed-type-replacement-cross-volume | cross-volume | rsync-a | 0.0398 | 6.8% | 0.0152 | 5,079,040 | 0 | baseline | baseline |
| mixed-type-replacement-cross-volume | cross-volume | xsync | 0.0262 | 1.8% | 0.0211 | 5,275,648 | 0 | 1.542 | yes |
| mixed-interrupted-resume-cross-volume | cross-volume | rsync-a | 0.0833 | 3.1% | 0.0723 | 5,390,336 | 0 | baseline | baseline |
| mixed-interrupted-resume-cross-volume | cross-volume | xsync | 0.1327 | 1.1% | 0.6343 | 5,406,720 | 0 | 0.630 | yes |
| deep-small-initial-copy-cross-volume | cross-volume | rsync-a | 0.2583 | 3.9% | 0.2371 | 5,259,264 | 0 | baseline | baseline |
| deep-small-initial-copy-cross-volume | cross-volume | xsync | 0.4558 | 8.1% | 2.7478 | 8,093,696 | 0 | 0.561 | yes |
| compressible-initial-copy-cross-volume | cross-volume | rsync-a | 0.0441 | 2.7% | 0.0217 | 5,210,112 | 0 | baseline | baseline |
| compressible-initial-copy-cross-volume | cross-volume | xsync | 0.0301 | 4.5% | 0.0957 | 4,259,840 | 0 | 1.525 | yes |
| incompressible-initial-copy-cross-volume | cross-volume | rsync-a | 0.0435 | 1.3% | 0.0205 | 5,177,344 | 0 | baseline | baseline |
| incompressible-initial-copy-cross-volume | cross-volume | xsync | 0.0339 | 4.0% | 0.0973 | 4,063,232 | 0 | 1.387 | yes |
| one-large-file-initial-copy-cross-volume | cross-volume | rsync-a | 0.0550 | 1.4% | 0.0234 | 5,292,032 | 0 | baseline | baseline |
| one-large-file-initial-copy-cross-volume | cross-volume | xsync | 0.0316 | 2.6% | 0.0113 | 3,850,240 | 0 | 1.748 | yes |
| mixed-initial-copy-pipe | pipe | rsync-a | 0.1559 | 2.9% | 0.1232 | 5,423,104 | 0 | baseline | baseline |
| mixed-initial-copy-pipe | pipe | rsync-az | 0.1496 | 1.5% | 0.1178 | 8,306,688 | 0 | 1.027 | yes |
| mixed-initial-copy-pipe | pipe | xsync | 0.1388 | 1.0% | 0.1322 | 7,667,712 | 915,733 | 1.099 | yes |
| mixed-initial-copy-pipe | pipe | xsync-raw | 0.1473 | 2.4% | 0.1341 | 7,110,656 | 1,794,073 | 1.029 | yes |
| mixed-no-op-second-sync-pipe | pipe | rsync-a | 0.0716 | 1.4% | 0.0204 | 5,095,424 | 0 | baseline | baseline |
| mixed-no-op-second-sync-pipe | pipe | rsync-az | 0.0627 | 10.4% | 0.0202 | 5,079,040 | 0 | 1.126 | yes |
| mixed-no-op-second-sync-pipe | pipe | xsync | 0.0390 | 3.8% | 0.0250 | 4,653,056 | 0 | 1.730 | yes |
| mixed-no-op-second-sync-pipe | pipe | xsync-raw | 0.0357 | 4.6% | 0.0257 | 4,505,600 | 0 | 1.924 | yes |
| mixed-content-churn-pipe | pipe | rsync-a | 0.0959 | 5.3% | 0.0426 | 5,619,712 | 0 | baseline | baseline |
| mixed-content-churn-pipe | pipe | rsync-az | 0.1012 | 2.4% | 0.0428 | 6,733,824 | 0 | 0.968 | yes |
| mixed-content-churn-pipe | pipe | xsync | 0.0664 | 0.4% | 0.0520 | 4,931,584 | 4,131 | 1.472 | yes |
| mixed-content-churn-pipe | pipe | xsync-raw | 0.0685 | 0.8% | 0.0533 | 4,751,360 | 20,633 | 1.434 | yes |
| mixed-metadata-only-churn-pipe | pipe | rsync-a | 0.0717 | 5.5% | 0.0216 | 5,390,336 | 0 | baseline | baseline |
| mixed-metadata-only-churn-pipe | pipe | rsync-az | 0.0725 | 2.5% | 0.0221 | 5,472,256 | 0 | 0.986 | yes |
| mixed-metadata-only-churn-pipe | pipe | xsync | 0.0401 | 3.0% | 0.0266 | 4,767,744 | 4,128 | 1.740 | yes |
| mixed-metadata-only-churn-pipe | pipe | xsync-raw | 0.0398 | 2.0% | 0.0272 | 4,620,288 | 20,633 | 1.782 | yes |
| mixed-delete-pipe | pipe | rsync-a | 0.0710 | 6.3% | 0.0225 | 5,095,424 | 0 | baseline | baseline |
| mixed-delete-pipe | pipe | rsync-az | 0.0706 | 2.8% | 0.0221 | 5,111,808 | 0 | 0.957 | yes |
| mixed-delete-pipe | pipe | xsync | 0.0393 | 4.6% | 0.0261 | 4,505,600 | 0 | 1.300 | yes |
| mixed-delete-pipe | pipe | xsync-raw | 0.0385 | 1.7% | 0.0269 | 4,571,136 | 0 | 1.806 | yes |
| mixed-type-replacement-pipe | pipe | rsync-a | 0.0702 | 2.7% | 0.0216 | 5,111,808 | 0 | baseline | baseline |
| mixed-type-replacement-pipe | pipe | rsync-az | 0.0721 | 1.8% | 0.0217 | 5,111,808 | 0 | 0.991 | yes |
| mixed-type-replacement-pipe | pipe | xsync | 0.0399 | 4.7% | 0.0280 | 4,587,520 | 0 | 1.745 | yes |
| mixed-type-replacement-pipe | pipe | xsync-raw | 0.0393 | 1.6% | 0.0274 | 4,521,984 | 0 | 1.757 | yes |
| mixed-interrupted-resume-pipe | pipe | rsync-a | 0.1140 | 4.1% | 0.0730 | 5,881,856 | 0 | baseline | baseline |
| mixed-interrupted-resume-pipe | pipe | rsync-az | 0.1097 | 4.7% | 0.0736 | 7,356,416 | 0 | 1.016 | yes |
| mixed-interrupted-resume-pipe | pipe | xsync | 0.0905 | 1.7% | 0.0810 | 6,455,296 | 694,544 | 1.206 | yes |
| mixed-interrupted-resume-pipe | pipe | xsync-raw | 0.0902 | 3.5% | 0.0781 | 5,963,776 | 894,602 | 1.264 | yes |
| deep-small-initial-copy-pipe | pipe | rsync-a | 0.2691 | 0.4% | 0.2339 | 5,210,112 | 0 | baseline | baseline |
| deep-small-initial-copy-pipe | pipe | rsync-az | 0.2596 | 2.8% | 0.2301 | 6,406,144 | 0 | 1.011 | yes |
| deep-small-initial-copy-pipe | pipe | xsync | 0.2961 | 1.0% | 0.2862 | 7,061,504 | 93,060 | 0.888 | yes |
| deep-small-initial-copy-pipe | pipe | xsync-raw | 0.3080 | 4.4% | 0.2930 | 7,028,736 | 112,860 | 0.869 | yes |
| compressible-initial-copy-pipe | pipe | rsync-a | 0.0641 | 7.1% | 0.0312 | 5,226,496 | 0 | baseline | baseline |
| compressible-initial-copy-pipe | pipe | rsync-az | 0.0797 | 0.8% | 0.0282 | 8,388,608 | 0 | 0.799 | yes |
| compressible-initial-copy-pipe | pipe | xsync | 0.0406 | 5.3% | 0.0281 | 6,684,672 | 2,944 | 1.549 | yes |
| compressible-initial-copy-pipe | pipe | xsync-raw | 0.0434 | 9.7% | 0.0281 | 6,488,064 | 2,098,816 | 1.636 | yes |
| incompressible-initial-copy-pipe | pipe | rsync-a | 0.0790 | 3.6% | 0.0274 | 5,193,728 | 0 | baseline | baseline |
| incompressible-initial-copy-pipe | pipe | rsync-az | 0.0545 | 3.3% | 0.0296 | 8,486,912 | 0 | 1.397 | yes |
| incompressible-initial-copy-pipe | pipe | xsync | 0.0413 | 4.4% | 0.0271 | 8,273,920 | 2,098,816 | 1.845 | yes |
| incompressible-initial-copy-pipe | pipe | xsync-raw | 0.0378 | 2.0% | 0.0249 | 6,012,928 | 2,098,816 | 2.128 | yes |
| one-large-file-initial-copy-pipe | pipe | rsync-a | 0.0846 | 3.5% | 0.0310 | 5,341,184 | 0 | baseline | baseline |
| one-large-file-initial-copy-pipe | pipe | rsync-az | 0.0852 | 2.0% | 0.0351 | 8,896,512 | 0 | 0.973 | yes |
| one-large-file-initial-copy-pipe | pipe | xsync | 0.0536 | 3.1% | 0.0380 | 42,385,408 | 8,388,660 | 1.386 | yes |
| one-large-file-initial-copy-pipe | pipe | xsync-raw | 0.0530 | 2.4% | 0.0367 | 39,321,600 | 8,388,660 | 1.589 | yes |
| ssh | ssh | *blocked* | - | - | - | - | - | - | the ssh route was not selected; native xsync-over-SSH and RsyncTransport rows require --routes ssh with --ssh-host |
