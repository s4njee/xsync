# congress-1m on orion: the gap is bytes ÷ disk speed

1,318,771 files, 13 GB, Mac → orion over router1, ext4 destination, measured to
durability, all files landed and counted. Binary `2823c2c246cc-dirty`.

| corpus | GB | files | rsync | xsync | gap | implied MB/s |
|---|---:|---:|---:|---:|---:|---:|
| `large1gb` | 0.847 | 4 | 8.39 s | 9.62 s | 1.23 s | 689 |
| `congress-1m` | 13.000 | 1,318,771 | 205.62 s | 222.34 s | 16.72 s | 778 |

## The gap is proportional to bytes, not fixed, and not per-file

- Bytes scale **15.3x**; the gap scales **13.6x**.
- Files scale **329,693x**; the gap scales **13.6x**.

Predicting the gap as `bytes ÷ 765 MB/s` — orion's measured raw sequential
write at PCIe Gen3 — gives **1.11 s** and **16.99 s** against observed **1.23 s**
and **16.72 s**: within 11% and 2% across a 15x range in size.

**xsync pays the full disk write time serially; rsync pays essentially none of
it.** That was the working model from tmpfs, Gen2/Gen3 and USB comparisons;
this is the first test that could separate it from per-file cost, and it does
so cleanly. A 330,000-fold increase in file count moves the gap not at all.

## Receiver memory, at the scale where it would show

| | receiver RSS peak |
|---|---:|
| rsync | 262.1 MB |
| xsync | **247.4 MB** |

At 1.3 million files on a 4 GB machine, **xsync uses less memory than rsync**.
The larger footprint xsync is known for is a *sender* phenomenon (298 MB on
congress-100k) and does not appear on the receiving side. Receiver memory is
now excluded as an explanation at both 4 files and 1.3 million.

This is also the first datapoint for 4.23's unmeasured-memory question on the
receiving end: 247 MB peak for a 1.3M-entry tree is comfortable on 4 GB. The
sender side remains unmeasured at this scale.

## Caveat

The bytes ÷ disk model explains the ext4 gap. It does **not** explain the
~0.35 s that persists on tmpfs, where disk time is zero. There are two
components; this pins the dominant one and leaves the smaller one open.

## Note on the harness

Run with a direct timing loop rather than `release-bench.py`: the committed
harness requires at least five repetitions for a gate, which at ~3.5 minutes
per arm is over an hour. The `--r0a-anchor` flag that permits three, along with
`--route-label` and the `large1gb` corpus, exist only in uncommitted work.
