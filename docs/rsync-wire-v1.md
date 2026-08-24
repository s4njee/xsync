# Rsync Receiver Wire Contract v1

Status: research contract for Story 4.4. This document defines the subset that
the Story 4.5 native `RsyncTransport` may implement. It is not a promise to
support every historical rsync extension.

## Dialects

| Dialect ID | Wire protocol | Implementations | v1 status |
|---|---:|---|---|
| `gnu-32` | 32 | GNU rsync 3.4.x and 3.5.x | Supported target |
| `apple-openrsync-29` | 29 | macOS `/usr/bin/rsync` | Conditional target |
| `openbsd-openrsync-27` | 27 | OpenBSD/openrsync, when installed | Research target |

The implementation must identify the remote implementation and negotiated
protocol before sending a file list. Unknown protocol versions, missing version
handshakes, and versions outside the selected matrix fail before destination
mutation. Protocol 32 is the preferred negotiation; protocol 29 is selected
only for the explicitly gated Apple openrsync dialect. Protocol 27 remains a
separate OpenBSD research target.

## Remote command

For local-to-remote whole-file sending, the remote process is launched as:

```text
rsync --server -logDtpre.iLsfxCIvu . DEST
```

The exact option letters are generated from the requested feature matrix; this
example is the observed GNU rsync command for archive-like, whole-file, dry-run
operation. The destination is passed as a separate remote-shell argument and
must never be interpolated into a shell command string.

The remote server speaks binary protocol on stdout. Stderr is diagnostic only
and remains separate from the binary stream. The local codec must treat a
non-zero remote exit, EOF before clean termination, and any multiplexed error
frame as failure.

## Common wire stages

1. Client and server exchange their maximum protocol integer. The negotiated
   protocol is the lower value; protocol 32 is observed on both research hosts.
2. The server sends a per-session checksum seed. The seed is a signed 32-bit
   wire integer and is nondeterministic; transcript fixtures represent it as
   `<seed>` and validate its position and use, not its literal value.
3. The sender transmits the exclusion list when the selected options require
   it, then a sorted file list terminated by the zero status byte.
4. The receiver/generator returns file indexes and phase changes. Whole-file
   mode sends no basis-file block signature and uses a literal data stream for
   each selected regular file.
5. The sender terminates each file with the negotiated whole-file checksum.
   Directory, symlink, and metadata records are handled through the file-list
   attributes and post-list update operations.
6. A clean end is a protocol phase completion followed by process exit zero;
   closing the byte stream early is not clean termination.

## Encoding rules

- Integers are little-endian signed 32-bit values unless the field is a
  protocol `long`; longs use the protocol's 32-bit value or marker followed by
  a signed 64-bit value.
- File-list names are byte strings with protocol-specific inherited-name and
  length flags. They are not UTF-8 strings.
- File-list entries are sorted lexicographically by relative path before index
  references are used.
- Most server-to-client bytes use the four-byte multiplex envelope. Tag `7`
  carries normal protocol data; other tags carry out-of-band messages. Tag `1`
  is a sender-side error and is fatal. The initial version/seed exchange is not
  multiplexed.
- Protocol 32 may negotiate checksum/compression names. v1 whole-file mode
  records the negotiated checksum and does not claim BLAKE3 semantics.

## v1 feature subset

Supported: regular files, nested and empty directories, symlinks, relative
paths, modes, mtimes, quick-check unchanged-file skipping, whole-file transfer,
and clean receiver errors.

Rejected before mutation unless separately implemented: delta-token transfer,
hardlinks, ownership, ACLs, xattrs, sparse/in-place behavior, compression,
`--delete`, arbitrary excludes, and checksum choices other than the negotiated
whole-file behavior. `--checksum` must either be mapped to the rsync checksum
algorithm or rejected; it must never be described as BLAKE3.

Rsync partial files and restart behavior are rsync behavior, not xsync durable
checkpoint resume. Multi-stream striping, xsync frame verification, and
`--paranoid` are unavailable on this backend.

## Provenance

- GNU rsync protocol and implementation: `rsync.samba.org/tech_report/`,
  `rsync.samba.org/how-rsync-works.html`, GNU rsync `rsync.1`, and the
  `RsyncProject/rsync` `rsync.h`, `compat.c`, and `csprotocol.txt` sources.
- Openrsync protocol and license boundary: OpenBSD `rsync(5)` and `openrsync(1)`
  manuals plus `kristapsdz/openrsync`. The OpenBSD implementation documents
  reference protocol 27; the Apple `/usr/bin/rsync` probe reports protocol 29
  and `rsync version 2.6.9 compatible`. The OpenBSD implementation is BSD/ISC
  licensed. No code from either implementation is copied or vendored.
- Local probes and normalized transcripts: `benches/results/story-4.4/`.
