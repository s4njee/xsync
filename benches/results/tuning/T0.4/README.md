# T0.4 phase timing evidence

Real five-repetition `congress-10k` same-volume smoke, source digest
`f5607e4b7af5d73f793730deabbf38071d28356a0f1eefe8f06e7f844e1380a6`.

The release report recorded timestamp-derived phase medians in JSON and Markdown:

| Phase | Median |
|---|---:|
| scan | 0.222221 s |
| plan | 0.036858 s |
| transfer | 5.654951 s |
| metadata | 0.000058 s |

All five rsync and five xsync oracle rows passed. The runner keeps seeding and oracle time
separate from the transfer phases and records an explicit `unaccounted` phase if the measured
phase sum differs from wall time by more than the allowed threshold.

## Plain-English result

The benchmark now shows where xsync spends its time instead of reporting one opaque duration. In
this real run, almost all xsync time was file transfer; scanning and metadata finalization were
small by comparison.
