# T7.1 worker sweep

Status: blocked pending the experiment harness and fixtures.

The existing `xsync-strategy-bench` covers synthetic dispatch at worker counts 1, 2, 4, 8,
and 16. It does not perform filesystem work. The local sync engine currently uses one internal
`local_workers` value for its file work, while directory and other metadata operations are not
exposed as a separately tunable phase. The command-line tool has no local-worker or
metadata-worker option.

The requested acceptance run now has a pinned `congress-100k` corpus, but still needs both APFS
and ext4 phase timing. The ext4 receiver is available in the project environment, but no
repeatable worker-sweep harness exposes separate metadata and data worker counts.

## Plain-English result

We cannot responsibly choose a worker default yet. The queue benchmark tells us that the
software can dispatch work with several worker counts, but it does not tell us whether more
filesystem workers make metadata or copying faster. Add separate phase controls and repeatable
APFS/ext4 fixtures, then rerun the 1–16 sweep before claiming a policy.
