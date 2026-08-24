# Story 4.3 — SSH startup and connection-model decision

## Question

What is the cost of establishing N parallel `xsync --server` sessions, relative to
transfer, and what connection model should xsync use so it never silently weakens
the user's SSH security posture or duplicates interactive authentication prompts?

## Evidence

Fresh measurement (this host, `xsync.connection-bench.v1`, 5 repetitions, median):

| streams | setup median (ms) | MAD (ms) | delta vs prev (ms) | transfer/setup |
|---:|---:|---:|---:|---:|
| 1 | 3.30 | 0.05 | 3.30 | 7.47 |
| 2 | 3.38 | 0.10 | 0.08 | 7.30 |
| 4 | 5.95 | 0.60 | 2.57 | 4.15 |
| 8 | 9.56 | 0.70 | 3.61 | 2.58 |

Reference transfer: 201 files, 1,867,776 B, ~24.7 ms.

The setup column measures spawning and v1-handshaking that many `xsync --server`
children in parallel over the pipe transport (the same `remote_server_command`
line a production host uses, minus the SSH round trip). Real SSH adds a roughly
constant per-session RTT + auth that the pipe path cannot reproduce without sshd;
the trend is what matters:

- Adding sessions is cheap in the *small* (1→2 adds ~0.1 ms because spawns
  overlap), then the cost recomposes superlinearly as CPU contention and the
  per-session handshake serialize (2→4 adds ~2.6 ms, 4→8 adds ~3.6 ms).
- At 8 sessions the setup time alone is ~40% of this small job's entire transfer
  time. Destination scans add a further per-session full tree walk, which is not
  included here and would only widen the crossover.

## Connection model

- **Default: one persistent `ssh {host} xsync --server` session per job.** xsync
  never spawns a long-lived master/ControlMaster socket of its own, never writes
  to the user's SSH config, and never downgrades host-key or authentication
  settings. Interactive password / host-key / keyboard-interactive prompts are
  read by OpenSSH from the controlling tty (`/dev/tty`), so the piped protocol
  stdin does not duplicate them. This is the whole of the shipped model (Story 4.1).
- **No multiplexing today.** Because a connection-control socket (e.g. `-M`/`-S`)
  is a security- and lifecycle-sensitive artifact, we do not create one unless a
  future change earns it: any such socket must live in an owned job directory
  with restrictive permissions, have deterministic cleanup, and fall back to
  ordinary persistent sessions on failure. None of that is implemented or claimed.
  This matches the plan's rule that no unverified session model is enabled.
- **Multi-stream (Story 4.2) stays off by default.** This measurement is the
  "measure before building" gate: N parallel sessions cost N handshakes plus N
  full destination scans, so striping only pays when a job's transfer phase
  dominates those costs (large files, many bytes). The crossover point — roughly
  where per-job setup is small relative to transfer — must be demonstrated for a
  given workload before `--streams N > 1` is enabled. For small/medium jobs that
  crossover is not met on this host, and `--streams` correctly resolves to one
  session (Story 0.5).

## Decision

1. Keep one persistent `ssh host xsync --server` session per job as the default
   and only shipped model.
2. Add neither implicit ControlMaster sockets nor any other silent connection
   multiplexing. If connection multiplexing is ever added it must be opt-in,
   socket-in-owned-dir, restrictive-perm, deterministic-cleanup, and
   persistent-session fallback.
3. From the crossover measured against the real Story 4.2 implementation
   (`xsync.stripe-bench.v1`, 5 reps, pipe-child = optimistic lower bound):

   | corpus | 4x/1x speedup |
   |---|---:|
   | single 4 MiB file | 0.95x |
   | single 16 MiB file | 1.35x |
   | single 64 MiB file | 1.84x |
   | many-small (1.6 MiB, 400 files) | 0.99x |

   Stripping is a *large-single-file* win: it crosses over between ~4 MiB and
   ~16 MiB per file and reaches ~1.8x at 64 MiB, while many-small and sub-cross
   jobs are flat-to-slightly-worse (setup + per-session overhead dominates; real
   ssh adds a per-session RTT only making this worse). Therefore:
   - `--streams` **defaults to 1** (Story 0.5) and is fully tested; the
     multi-stream path is provisional opt-in.
   - An explicit `--streams N > 1` is honored within 1..=16 and pays only when a
     job is dominated by a few very large files; it is not a speedup for
     small/medium or many-small workloads, which should stay at one stream.

4. Multi-stream (Story 4.2) is gated accordingly: its correctness path is tested
   (a 64 MiB file striped across four sessions is byte-identical), but it is not
   the default and is not claimed as a universal multiplier — matching the
   evidence-driven policy established in plan.md.

(1)–(3) restate the connection model and measurement; (4) is the concrete
enablement rule this loop closes.