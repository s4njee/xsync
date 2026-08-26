# T4.1 evidence

The full five-repetition `congress-1m` run is at `/tmp/xsync-t4.1-1m/report.md` and
`/tmp/xsync-t4.1-1m/report.json`. The pinned manifest digest is
`0332417e2c92df6f27209ae0a84318c2eff1c7a4227dec1bc78dbfbcc592e7a5` and the scan contained
1,787,545 entries.

Full-scale medians:

| Metric | Median |
|---|---:|
| Destination index build | 12.963047 s |
| Planner | 2.697094 s |
| Scan phase | 26.061453 s |
| Peak RSS | 1,091,600,384 bytes |

The 512 MiB memory budget was exceeded, and the required under-one-second plan was not met.
Correctness held for all five repetitions (`1,787,545` planned items each).

Plain English: the index prototype works on the large tree, but it currently costs about 1.09 GB
of memory and several seconds of planning. That is useful diagnostic evidence, not a reason to
integrate it yet.

## Earlier lower-scale evidence

The available worker sample was run against `congress-10k` (`/Users/sanjee/projects/csearchv2/congress/data/100`):

```text
item_count=22567
destination_index_seconds=0.208983833
source_scan_seconds=0.14126975
planner_seconds=0.019240625
queue_high_water=1024
```

The five-repetition report could not be emitted because macOS `/usr/bin/time -l` exits with:
`time: sysctl kern.clockrate: Operation not permitted`. The worker therefore cannot provide the
required peak-RSS measurement in this environment. The `congress-1m` corpus is also unavailable,
so this is diagnostic evidence only and not T4.1 acceptance evidence.
