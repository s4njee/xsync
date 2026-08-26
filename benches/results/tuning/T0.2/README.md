# T0.2 oracle cost measurement

The full independent oracle was run against the live `congress-1m` source and its read-only
manifest.

| Item | Result |
|---|---:|
| Manifest items | 1,787,546 |
| Regular files hashed | 1,318,771 |
| Logical bytes | 10,987,200,342 |
| Mismatches | 0 |
| Full verification wall time | 282.17 s |
| Manifest capture wall time | 292.49 s |

The verification JSON is `/tmp/xsync-t0-congress-1m-verify.json` for this run. The runner’s
sampling mode remains explicit and deterministic: metadata is always checked, sampled content
hashes carry their fraction and seed, and repetition 1 remains full.

## Plain-English result

Checking this 1.3-million-file tree completely takes about 4.7 minutes on this machine. Sampling
is therefore useful for later repetitions, but it must remain visibly labelled as sampled; the
first repetition should stay a full correctness check.
