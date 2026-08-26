# T3.1 clone/per-file crossover report

Status: complete for the pinned `congress-100k` corpus.

Source: `/Users/sanjee/projects/csearchv2/congress/data/118`  
Manifest: `2242c0ea6a327de9e476114185e37b7215f0d9157107e404a7a7a63b3d5fe794`  
Manifest contents: 135,466 items, 109,615 files, 583,940,018 logical bytes.

The experiment populated a disposable destination with the source tree, removed one complete
subtree, and ran either the directory-clone path or the benchmark-only `--no-directory-clone`
per-file path. The rotated block ran per-file first and clone second for three repetitions at
each size. The final destination passed the independent full manifest oracle; every rotated cell
reported zero failed entries.

## Crossover bracket

| Missing subtree | Logical bytes | Files | Per-file median (MAD) | Clone median (MAD) | Winner |
|---|---:|---:|---:|---:|---|
| `bills/hconres` | 4,062,586 | 675 | 2.22 s (0.12) | **2.06 s (0.01)** | clone |
| `bills/hjres` | 7,328,529 | 1,150 | **2.12 s (0.02)** | 2.40 s (0.01) | per-file |
| `bills/sres` | 17,729,567 | 4,710 | **2.72 s (0.10)** | 3.04 s (0.10) | per-file |
| `votes` | 115,680,459 | 3,864 | **2.62 s (0.01)** | 3.19 s (0.00) | per-file |
| `bills/hr` | 284,933,886 | 52,820 | **10.26 s (0.05)** | 14.67 s (0.15) | per-file |

The observed crossover is between 4.06 MB and 7.33 MB on this APFS host and implementation.
The table is a measured bracket, not a universal threshold for other filesystems or hardware.

## Phase timing

Median seconds with MAD in parentheses, from timestamped progress events in the rotated block:

| Subtree | Mode | Scan | Plan | Transfer | Metadata |
|---|---|---:|---:|---:|---:|
| hconres | per-file | 1.546 (0.091) | 0.244 (0.005) | 0.100 (0.000) | 0.006 (0.000) |
| hconres | clone | 1.343 (0.015) | 0.230 (0.005) | 0.160 (0.002) | 0.000 (0.000) |
| hjres | per-file | 1.383 (0.005) | 0.238 (0.002) | 0.173 (0.001) | 0.010 (0.000) |
| hjres | clone | 1.568 (0.037) | 0.253 (0.011) | 0.279 (0.004) | 0.000 (0.000) |
| sres | per-file | 1.412 (0.070) | 0.240 (0.002) | 0.700 (0.007) | 0.034 (0.001) |
| sres | clone | 1.374 (0.022) | 0.226 (0.005) | 1.116 (0.015) | 0.000 (0.000) |
| votes | per-file | 1.357 (0.015) | 0.224 (0.002) | 0.649 (0.001) | 0.069 (0.001) |
| votes | clone | 1.393 (0.043) | 0.217 (0.004) | 1.227 (0.013) | 0.000 (0.000) |
| hr | per-file | 1.241 (0.088) | 0.246 (0.027) | 8.172 (0.043) | 0.451 (0.016) |
| hr | clone | 1.243 (0.083) | 0.233 (0.001) | 13.235 (0.050) | 0.000 (0.000) |

## Non-reflink fallback

On ext4 at `192.168.1.119`, the 100k `bills/hconres` case completed in 2.06 seconds with
`directory_clones=0`, `byte_copies=675`, and `wire_bytes=857831`. The remote full oracle passed
all 135,466 items with zero mismatches and the pinned digest.

## Plain-English result

For this Mac, cloning is worthwhile for a missing directory around 4 MB, but ordinary per-file
copying wins by about 7 MB and remains better for the larger tested directories. The program now
chooses the largest missing subtree safely, verifies the result, and falls back to ordinary copies
on ext4 when cloning is unavailable.
