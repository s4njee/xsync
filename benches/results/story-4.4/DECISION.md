# Story 4.4 Decision

## Evidence

Read-only probes on 2026-08-23:

| Host | Implementation | Version | Protocol | openrsync |
|---|---|---:|---:|---|
| `sanjee@mars.local` | GNU rsync | 3.4.4 | 32 | unavailable |
| `sanjee@freya.local` | GNU rsync | 3.5.0-g471e17dc | 32 | unavailable |
| local `/usr/bin/rsync` | Apple openrsync-compatible | 2.6.9 compatible | 29 | present |

Both hosts completed a dry-run whole-file probe. The observed remote command
shape was `rsync --server ... . /tmp/xsync-rsync-probe-dst/`; the command was
shown by GNU rsync's verbose client output and no destination mutation was
requested. The local Apple client also completed a dry-run against Mars with
`--protocol=29`; it reported `client version 29, server version 32, negotiated
protocol version 29` and exited successfully. Neither Linux host has an
`openrsync` executable, so the Apple client is validated here only as a client,
not as a remote receiver.

## Decision

1. Story 4.5 targets GNU protocol 32 first, with Apple openrsync protocol 29 as
   a separately gated dialect. OpenBSD protocol 27 remains a research target.
2. Unknown implementations and protocol versions are rejected before the file
   list and before destination mutation. A protocol number alone is not enough
   to claim compatibility when the implementation reports unsupported feature
   flags.
3. v1 implements local-to-remote whole-file sending only. Delta-token generation
   is explicitly deferred; the receiver may send an empty basis signature in
   whole-file mode.
4. The normalized transcript contract in `transcripts-v1.md` is the fixture
   boundary. It preserves nondeterministic checksum seeds as `<seed>` while
   checking their field position and downstream checksum use.
5. Apple openrsync remains conditional as a receiver until a macOS host can
   provide a server-side integration probe. Its protocol-29 client handshake is
   recorded, but it is not represented as GNU protocol-32 compatibility.

## Consequences

The future codec must keep binary stdout isolated from diagnostics, use checked
length/count allocation, preserve raw path bytes, and expose the negotiated
implementation/version/protocol in events. It must not execute a local rsync
binary in production.
