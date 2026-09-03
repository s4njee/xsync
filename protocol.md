# xsync Native Sync Protocol v2

This document is the compatibility contract for the native sync v2 remote protocol. The
Rust types in `xsync-core::protocol` implement this layout; Rust enum layout or
the choice of serialization library is not observable on the wire.

The canonical ownership and review process for both xsync and f2 is in
[`docs/protocol-ownership.md`](docs/protocol-ownership.md). Changes to this
document are not complete until the process there and the compatibility matrix
have been followed.

## Envelope

Every frame begins with this fixed 32-byte little-endian header, followed by
exactly `payload_len` bytes:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic, ASCII `xsn1` |
| 4 | 2 | header length, exactly `32` |
| 6 | 2 | reserved, zero |
| 8 | 4 | protocol version, exactly `2` |
| 12 | 1 | message type |
| 13 | 1 | flags, bit 0 is zstd compression |
| 14 | 2 | reserved, zero |
| 16 | 4 | encoded payload length |
| 20 | 4 | decoded payload length |
| 24 | 8 | unique message ID |

All integers are unsigned unless stated otherwise and use least-significant
byte first. v1 rejects a different magic, version, header length, reserved
field, message type, or flag. The version error is exactly:
`xsync version mismatch: local vX / remote vY`.

The encoded and decoded payload lengths are each capped at 16 MiB. For an
uncompressed frame they must be equal. A compressed frame is decompressed only
to its declared decoded length, which is also capped at 16 MiB. A receiver
checks these limits and checked `header + payload` arithmetic before allocating
or reading the body. In-memory decoding rejects trailing bytes; stream decoding
consumes one complete frame and leaves the next frame unread.

Message IDs are unique within a session. `FrameDecoder` retains a bounded set
of disjoint received-ID ranges and rejects duplicates before dispatch. A large
sequential transfer therefore remains bounded without imposing a frame-count
limit; a hostile stream with more than 1,048,576 disjoint ranges is rejected by
the session budget.

## Payload encoding

The payload begins directly with the fields for the envelope message type. It
has no serializer-specific tag. Integers remain little-endian. Length-prefixed
byte strings use a `u32` byte length followed by exactly that many bytes.
Paths are raw bytes, allowing non-UTF-8 names; the maximum encoded path is 1
MiB. Error messages are UTF-8 and capped at 64 KiB. Booleans are exactly `0`
or `1`; unknown enum values and any typed-payload trailing bytes are errors.

Collection counts are `u32` and are capped at 65,536 records per frame. Resume
pages are capped at 65,536 ranges. The receiver checks every count before
reserving collection capacity. A logical 32 MiB batch or 16 MiB large-file
chunk is therefore represented by multiple bounded frames, never one oversized
frame.

## Message types

The type byte assignments are frozen:

| Type | Message | Payload fields in order |
|---:|---|---|
| 1 | Handshake | role `u8`, capabilities `u32`, max payload `u32`, max segment `u32`, window `u32`, job ID `[16]`, compression `u8` |
| 2 | SessionConfig | streams `u8`, batch bytes `u32`, chunk bytes `u32`, window `u32`, delete/checksum/paranoid booleans |
| 3 | FileBatch | batch ID `u64`, count, entry records |
| 4 | FileSegment | file ID `u64`, offset `u64`, segment digest `[32]`, data bytes |
| 5 | LargeFilePrepare | file ID `u64`, path, size `u64`, mtime ns `i64`, mode `u32`, fingerprint `[32]` |
| 6 | LargeFileRange | file ID `u64`, range offset `u64`, range length `u64` |
| 7 | LargeFileFinish | file ID `u64`, digest `[32]` |
| 8 | Metadata | operation `u8`, path, symlink target path, mode `u32`, mtime ns `i64` |
| 9 | Scan | scan ID `u64`, final-page boolean, count, entry records |
| 10 | Stats | files, bytes, skipped, warnings, failed `u64` values |
| 11 | Ack | acknowledged ID `u64`, acknowledged type `u8` |
| 12 | Error | code `u16`, related ID `u64`, UTF-8 message |
| 13 | ResumePage | file ID `u64`, page `u32`, final-page boolean, count, ranges |

An entry record is: path, kind `u8`, size `u64`, mtime ns `i64`, mode `u32`,
fingerprint `[32]`. A byte range is offset `u64` and length `u64`; ranges must
be non-empty, no larger than 8 MiB, checked for overflow, and sorted and
non-overlapping in a resume page. `RangeTracker` additionally checks file
bounds and overlap across received pages.

Data segments and large-file ranges are capped at 8 MiB. The default
unacknowledged window is 32 MiB; handshake and session configuration values
must be non-zero and no larger than that limit. Handshake payload and session
configuration have additional fixed maximums of 128 and 32 bytes.

## Compatibility and errors

v1 is fail-closed. A receiver never silently ignores unknown types, flags,
fields, enum values, trailing data, duplicate IDs, overlapping ranges, or
length violations. A future incompatible layout must increment the version;
it must not reinterpret a v1 field. A future compatible message type requires a
new version because v1 rejects unknown types.

Capability bits are carried in the `Handshake.capabilities: u32` field and are
deliberately un-masked, so a receiver that does not recognize a bit simply
ignores it. `CAP_DATA_ONLY = 1 << 0` requests a data-only receiver session for
multi-stream (Story 4.2): the server skips the destination scan, rejects
metadata/large-file-finish messages, and only accepts `FileBatch`/`FileSegment`
and `LargeFilePrepare`/`LargeFileRange`/`FileSegment` traffic against the shared
stage. A server that does not understand the bit still answers as an ordinary
single-session sink, so an old peer against a new client degrades to single
stream rather than failing.

`CAP_ZSTD = 1 << 1` advertises zstd support. The negotiated compression mode is
the intersection of the requested mode and both peers' capability bits. If
either peer lacks `CAP_ZSTD`, the session uses `None`, and this choice is made
during the handshake before any data frame is sent.

## v2 message table

Protocol v2 uses the same 32-byte envelope shape and `xsn1` magic. The envelope
version is `2` after the handshake selects v2. The v1 opening handshake and its
acknowledgement remain v1 envelope frames as specified by
[`v2handshake.md`](v2handshake.md). V1 message types 1 through 13 retain their
payload layouts; new v2 message types begin at 14.

The v2 reader remains fail-closed. It accepts only the types listed below,
checks every bound before allocation, and rejects trailing payload bytes. A v1
peer never receives these types because the handshake selects v1 before
`SessionConfig`.

The consuming implementations are xsync and f2. A change to this table is a
protocol change and must update both consumers or add a versioned amendment;
implementation enum order is never the wire contract.

The v1 handshake role values are `1` source, `2` sink, and `3` long-lived
session. Role `3` requires the v2 negotiation and browse capability bits; it
never enters the v1 sync state machine.

| Type | Message | Payload fields in order |
|---:|---|---|
| 14 | ListRequest | path, page token `u64`, page size `u32` |
| 15 | ListPage | related request ID `u64`, page token `u64`, final-page boolean, count, entries |
| 16 | StatRequest | path, include-digest boolean |
| 17 | StatResponse | related request ID `u64`, status `u8`, entry record, optional digest `[32]`, error message |
| 18 | CancelRequest | related request ID `u64` |
| 19 | Keepalive | nonce `u64` |
| 20 | KeepaliveAck | nonce `u64` |
| 21 | BrowseError | related request ID `u64`, code `u16`, UTF-8 message |
| 22 | RenameRequest | source path, destination path |
| 23 | RenameResponse | related request ID `u64`, mutation status `u8`, error message |
| 24 | CreateDirectoryRequest | path |
| 25 | CreateDirectoryResponse | related request ID `u64`, mutation status `u8`, error message |
| 26 | DeleteRequest | path |
| 27 | DeleteProgress | related request ID `u64`, path, removed boolean, error message |
| 28 | DeleteResponse | related request ID `u64`, delete status `u8`, removed count `u64`, irreversible boolean, failure collection |
| 29 | FetchRequest | path |
| 30 | FetchStart | related request ID `u64`, size `u64`, mtime ns `i64`, device `u64`, file `u64`, BLAKE3 digest `[32]` |
| 31 | FetchChunk | related request ID `u64`, offset `u64`, data |
| 32 | PublishRequest | path, fetched size `u64`, fetched mtime ns `i64`, fetched device `u64`, fetched file `u64`, replacement size `u64`, replacement digest `[32]` |
| 33 | PublishReady | related request ID `u64` |
| 34 | PublishChunk | related request ID `u64`, offset `u64`, data |
| 35 | PublishResponse | related request ID `u64`, publish status `u8`, current identity, error message |
| 36 | SetPermissionsRequest | path, mode `u32` |
| 37 | SetPermissionsResponse | related request ID `u64`, mutation status `u8`, error message |
| 38 | SetMtimeRequest | path, mtime ns `i64` |
| 39 | SetMtimeResponse | related request ID `u64`, mutation status `u8`, error message |
| 40 | ReadLinkRequest | path |
| 41 | ReadLinkResponse | related request ID `u64`, stat status `u8`, symlink target, error message |

The envelope message ID remains unique for every frame. Response messages also
carry `related request ID` because a long-lived session may have more than one
request in flight and the response's own envelope ID is not the request ID.
The server processes requests in arrival order for v2. A cancellation applies
only to the related request and is acknowledged by either the request's normal
final response or a `BrowseError` with the cancellation code.

### v2 field encoding and bounds

- `path` and symlink targets are raw byte strings with a maximum encoded length
  of 1 MiB. Paths are relative to the configured server root; `ListRequest`
  accepts the empty path for the root. A list entry name is one relative path
  component and uses the same 1 MiB maximum.
- `page token` is an opaque server-issued cursor. Zero requests the first page;
  a non-final page returns the token for the next page. Tokens are scoped to the
  related request and must not be reused after the final page.
- `page size` is bounded to `1..=65,536`. The server may return fewer entries
  than requested without marking the page final.
- Collection counts are `u32` and capped at 65,536 before reserving memory.
- Each list/stat entry is encoded in this order: name/path blob, kind `u8`,
  size `u64`, mtime ns `i64`, mode `u32`, symlink-target blob. The target blob
  is empty unless kind is symlink. `ListPage` repeats this entry structure
  exactly `count` times.
- File sizes are `u64`, mtimes are signed nanoseconds `i64`, modes are `u32`,
  and kinds use the frozen v1 wire values: `1` file, `2` directory, `3` symlink,
  and `4` other. Symlink entries include a raw target; regular files,
  directories, and other entries encode an empty target.
- `StatResponse.status` is `ok`, `missing`, or `error`. `missing` is a normal
  negative answer and carries no entry. For `ok`, the entry follows the status,
  then a digest-present boolean and the optional digest. For `missing`, only a
  digest-present boolean with value false follows. For `error`, the entry and
  digest are absent and the bounded UTF-8 error message follows.
- `include-digest=true` permits a BLAKE3 digest only for a regular file. The
  digest is omitted for directories, symlinks, missing paths, and errors.
- `BrowseError` messages are UTF-8 and capped at 64 KiB. Its code values are
  frozen as `1 cancelled`, `2 invalid request`, `3 permission denied`,
  `4 unavailable`, and `5 internal error`.
- Rename refuses an existing destination and uses the filesystem rename operation,
  preserving its atomicity. Mutation responses use status values `0 ok`,
  `1 already exists`, `2 permission denied`, `3 parent missing`, `4 cross-device`
  (`EXDEV`, rename only), and `5 error`. A non-zero status carries a bounded
  UTF-8 error message; status `ok` carries no message.
- `CreateDirectoryRequest` creates exactly one directory and does not create
  missing parents. Both mutation requests and responses are request-scoped and
  use the same path and error bounds as browse requests.
- `DeleteRequest` is irreversible and recursively removes the requested path.
  It never follows symlinks. `DeleteProgress` is emitted once per attempted
  item, including failures. Delete status values are `0 complete`, `1 partial`,
  and `2 cancelled`; a response always sets `irreversible=true` and includes
  every failed path with its signed platform errno. A directory changed during
  traversal is handled as a snapshot: entries observed after a directory is
  read are not part of the operation, while removal failures for entries that
  disappear or become non-empty are reported in the final failure list.
- `FetchRequest` accepts only a regular file. The server performs the existing
  stable source read, sends one `FetchStart`, then ordered chunks of at most 1
  MiB. The start metadata and digest describe the exact bytes in those chunks;
  a failed stable read is a request-scoped error and sends no data.
- `PublishRequest` is accepted only when the target's current size, mtime, and
  filesystem identity equal the fetched identity. Replacement size and digest are independent of
  that identity, so an edit may change both file length and content. A mismatch returns status
  `changed` and the current identity, including an explicit absent identity.
  Status values are `0 ok`, `1 changed`, and `2 error`. After `PublishReady`,
  ordered chunks are verified by size and BLAKE3, staged through the sink's
  deterministic temporary path, and atomically renamed into place.
- Keepalive frames carry no filesystem state and are valid only while the
  session is established. An unknown nonce is a protocol error.
- Types 36–41 (`SetPermissions`, `SetMtime`, `ReadLink`) are gated on
  `CAP_BROWSE_META` (`1 << 6`). A peer that does not advertise the bit must
  never be sent these types: v2 is fail-closed on an unknown type byte, so an
  older `xs` would abort the session. The bit is ignored by an older handshake
  decoder, which is why the older peer degrades rather than errors. A server
  that does not advertise the bit and still receives one of these types treats
  it as an unexpected session message.
- `SetPermissionsRequest` carries a Unix mode `u32`; the server applies
  `mode & 0o7777`. `SetMtimeRequest` carries signed nanoseconds. Both follow a
  final symlink, matching SFTP `SETSTAT`. Intermediate symlink components are
  rejected as a traversal escape, as with `StatRequest`.
- `ReadLinkRequest` does not follow the final symlink. Status values reuse the
  frozen `StatResponse` codes (`0 ok`, `1 missing`, `2 error`). `ok` carries
  the raw target blob (empty is a valid target); `missing` carries nothing;
  `error` carries a bounded UTF-8 message and is used both for a non-symlink
  and for a filesystem failure. A non-zero mutation status on types 37 and 39
  carries a bounded UTF-8 error message; status `ok` carries no message, as
  with rename and mkdir.
- The existing 16 MiB encoded and decoded payload caps, checked length
  arithmetic, reserved-field rules, and compression rules apply unchanged.

### v1 and v2 message handling

- A v2 session accepts the v1 `Ack` and `Error` frames required to complete the
  opening handshake. After the selected-version boundary, it accepts only the
  v2 table above plus v1 transfer messages explicitly reused by a later
  story's transfer contract.
- A v1 session accepts no type above 13 and must never be sent one. A v2 server
  communicating with a v1 client selects v1 and uses only the frozen v1 table.
- V2 does not reinterpret a v1 message with a new payload. If a future feature
  needs different fields, it receives a new type or a new protocol version.
- Browse errors are per-request and do not terminate the session unless the
  error is a framing, bounds, duplicate-ID, or other protocol error.

This table is the v2 freeze for Stories 9.1, 9.2, 11.1, 12.1, and 12.2,
amended with types 36–41 (`CAP_BROWSE_META`) for Kestrel XS-B2. Additional
session controls receive new type assignments in a later protocol revision or
an explicitly amended v2 table; they must not reuse these numbers. The f2
copied-vector fixture has not been updated for types 36–41; that consumer
change is an explicit blocker for an xsync *release* that advertises
`CAP_BROWSE_META`, not for Kestrel consuming a path-dep. An f2 build without
the types never advertises the bit and never sends them.

Malformed compression, a compressed output larger than the declared bounded
length, or a declared decompressed length over 16 MiB is rejected before any
filesystem publication. Protocol decoding does not write destination files;
the sink may consume only fully decoded and separately verified operations.

## v3 message table

Protocol v3 is the random-access filesystem surface specified in `xsyncv3.md`.
It uses the same 32-byte envelope shape and `xsn1` magic; the envelope version
is `3` after the handshake selects v3. The v1 opening handshake and its
acknowledgement remain v1 envelope frames as specified by `v2handshake.md`.
Selection requires both peers to advertise `CAP_VERSION_NEGOTIATION` and
`CAP_FS_V3` (`1 << 7`); otherwise the v2 rule applies unchanged. A v3 client
against a v2 server therefore gets browse v2 with reduced capability, and a v2
client never receives a v3 frame.

The v3 reader is fail-closed in exactly the way the v2 reader is: it accepts
only the types below, checks every bound before allocation, rejects unknown
flag and presence bits, rejects field pairs that disagree, and rejects
trailing payload bytes. A v3 selection is never inferred from a v2 decode
failure and a v3 decode failure never restarts negotiation.

This table is the **Phase 1 freeze** (`xsyncv3.md` §5): the types a client
needs to mount, open, read, write, stat, list and measure capacity. Later
phases add types in the reserved ranges below; they never change a type
listed here. The consuming implementations are xsync and Excalibur; f2 does
not implement v3.

### Shared control types

Types `18 CancelRequest`, `19 Keepalive` and `20 KeepaliveAck` are accepted in a
v3 session with their v2 payload layouts unchanged. Cancellation applies to
every v3 request type, including an in-flight `Read` or `ReadDir`; the server
stops sending and answers with `Error` code `17 ECANCELED` or the request's
normal terminal response.

### Types

| Type | Message | Payload fields in order |
|---:|---|---|
| 42 | Features | features `u64` |
| 43 | FeaturesAck | related request ID `u64`, features `u64` |
| 50 | Mount | export path, requested access `u8` |
| 51 | MountInfo | related request ID `u64`, export path, access `u8`, effective-writable boolean, reason UTF-8, options UTF-8, case-sensitive boolean, normalization `u8`, max name length `u32`, max path length `u32`, supports `u64`, max read `u32`, max write `u32`, attribute cache ms `u32`, directory cache ms `u32` |
| 56 | Open | path, flags `u32`, mode `u32`, attr mask `u32` |
| 57 | Opened | related request ID `u64`, handle `u64`, attrs record |
| 58 | Close | handle `u64` |
| 59 | Read | handle `u64`, offset `u64`, length `u32`, want-digest boolean |
| 60 | ReadData | related request ID `u64`, offset `u64`, eof boolean, digest-present boolean, optional digest `[32]`, data bytes |
| 61 | Write | handle `u64`, offset `u64`, digest-present boolean, optional digest `[32]`, data bytes |
| 62 | WriteAck | related request ID `u64`, bytes written `u32`, new size `u64`, stable boolean, change cookie `[16]` |
| 63 | Flush | handle `u64` |
| 80 | Stat | target `u8`, path, handle `u64`, follow boolean, attr mask `u32` |
| 81 | Attrs | related request ID `u64`, attrs record |
| 82 | ReadDir | handle `u64`, cursor `u64`, max entries `u32`, attr mask `u32` |
| 83 | DirPage | related request ID `u64`, cursor `u64`, final-page boolean, count, entries |
| 84 | StatFs | *(no fields)* |
| 85 | FsInfo | related request ID `u64`, block size `u32`, total bytes `u64`, free bytes `u64`, available bytes `u64`, total inodes `u64`, free inodes `u64`, fs type UTF-8, max name length `u32`, case-sensitive boolean, normalization `u8`, read-only boolean |
| 70 | StageOpen | destination path, size `u64`, digest-present boolean, optional digest `[32]`, mode `u32`, resume token |
| 71 | StageOpened | related request ID `u64`, stage ID `u64`, resume token, staged bytes `u64` |
| 72 | StageWrite | stage ID `u64`, offset `u64`, digest-present boolean, optional digest `[32]`, data bytes |
| 73 | StageAck | related request ID `u64`, bytes written `u32`, staged bytes `u64` |
| 74 | StageStatus | stage ID `u64`, cursor `u64` |
| 75 | StageRanges | related request ID `u64`, cursor `u64`, final-page boolean, count `u32`, then per range start `u64`, end `u64` |
| 76 | StageCommit | stage ID `u64`, digest `[32]`, cookie-present boolean, optional expect cookie `[16]`, mtime-present boolean, optional mtime ns `i64` |
| 77 | StageResult | related request ID `u64`, outcome `u8` (0 committed, 1 changed), attrs |
| 78 | StageAbort | stage ID `u64` |
| 79 | WriteCas | handle `u64`, offset `u64`, expect cookie `[16]`, digest-present boolean, optional digest `[32]`, data bytes |
| 86 | Rename | source path, destination path, mode `u8` (0 `NOREPLACE`, 1 `REPLACE`, 2 `EXCHANGE`), attr mask `u32` |
| 87 | Unlink | path |
| 88 | Rmdir | path |
| 89 | Mkdir | path, mode `u32`, attr mask `u32` |
| 90 | Symlink | target, path, attr mask `u32` |
| 91 | Link | existing path, new path, attr mask `u32` |
| 92 | Chown | *stat target*, uid present boolean, uid `u32`, gid present boolean, gid `u32`, follow boolean, attr mask `u32` |
| 93 | SetTimes | *stat target*, atime *time change*, mtime *time change*, follow boolean, attr mask `u32` |
| 94 | SetPermissions | *stat target*, mode `u32`, follow boolean, attr mask `u32` |
| 95 | Mutated | related request ID `u64`, attrs |
| 121 | Error | related request ID `u64`, code `u16`, platform errno `i32`, UTF-8 message |
| 122 | Done | related request ID `u64` |

`StageRanges` carries half-open `[start, end)` pairs, ascending, disjoint and
non-empty. A set that overlaps or runs backwards is rejected: it describes no
file, and accepting one would let a peer make a resuming client skip bytes it
never sent.

Types 70–79 are staged uploads and compare-and-swap. A stage is a temporary file
beside its destination plus a **sidecar recording which ranges it holds**, so it
outlives the connection that created it and the server process itself: a client
reconnects on a new session, hands back the resume token, and continues.
Session resume (types 52–55) is a different mechanism for a different problem
and this does not depend on it. `StageCommit` verifies the whole-file digest,
refuses a stage with gaps, applies mode and mtime to the temporary file, and
then renames — so the destination never exists in a partial state.

`expect_cookie` is the compare-and-swap. It is checked immediately before the
rename, not at `StageOpen`: a check taken before the bytes are transferred
proves nothing about the moment of publication. An **all-zero cookie means
"create, and only create"** — the case v2's `Publish` could not express, because
it answered `Changed` for a destination that did not exist. A refused commit is
a `StageResult` with outcome `1`, not an `Error`: the caller needs the
destination's current attributes to decide what to do, and an enum has no
default arm that a caller can misread as success.

`WriteCas` is the same guard for a small in-place edit that does not justify a
stage. `Write` could not take an optional cookie because type 61 is in the
frozen Phase 1 table. The check happens immediately before the write, inside the
handle's exclusive ordering domain; **it is not a lock**, because another handle
can still write between the check and the write. Anything that must be atomic
against a concurrent writer wants a stage, whose publication is a rename.

A *stat target* is the three fields `Stat` uses: tag `u8` (0 path, 1 handle),
path, handle `u64`. Both fields are always present and the unused one must be
zero or empty; a payload that fills both is rejected rather than resolved by
preference.

A *time change* is tag `u8` (0 `UTIME_OMIT`, 1 `UTIME_NOW`, 2 set), seconds
`i64`, nanoseconds `u32` — **always all three**, whatever the tag, so the frame
layout never depends on a value the peer chose. Nanoseconds must be less than
1 000 000 000. `UTIME_NOW` is resolved by the server, whose clock is the one the
filesystem stamps with.

Types 86–94 are the mutations. Every one of them is refused with `EROFS` on a
read-only mount before it reaches the filesystem. Those that leave something
behind answer `Mutated` with the new attributes, so a client gets the fresh
change cookie without a second round trip; `Unlink` and `Rmdir` answer `Done`,
because there is nothing left to describe. `Rename` returns `EXDEV` across
filesystems rather than falling back to a server-side copy: that would turn one
atomic request into a long one that can half-succeed, and only the client can
decide whether that is acceptable. A `Rename` whose two paths resolve to the
same file succeeds and does nothing, as POSIX `rename` does.

Reserved for later phases and never reused: 44–49 (authentication and export
discovery), 52–55 (session resume, unmount, `MountChanged`), 64–69 (`SetSize`,
`Allocate`, `Seek`, `Advise`, `HandleState`), 96–99
(`Access`/`AccessResult`, and two spare), 100–109 (locks and leases), 110–115
(watches), 116–119 (compound), 120 (`Shutdown`), 123–127 (extended attributes —
moved out of 86–99, which was assigned before anyone counted and holds four
fewer types than the group needs).

### The attrs record

Every `Attrs`, `Opened` and `DirPage` entry carries this record. It begins with
a presence bitmap so optional blocks cost nothing when absent.

```text
presence u32
kind u8            1 file, 2 directory, 3 symlink, 4 other (frozen v1 values)
mode u32           permission bits, at most 0o7777
size u64
mtime ns i64
change cookie [16] opaque; equal cookies mean "unchanged"
```

followed, in bit order, by each block whose presence bit is set:

| Bit | Block | Fields |
|---:|---|---|
| 0 | owner | uid `u32`, gid `u32` |
| 1 | nlink | nlink `u32` |
| 2 | atime | atime ns `i64` |
| 3 | ctime | ctime ns `i64` |
| 4 | btime | btime ns `i64` |
| 5 | identity | dev `u64`, ino `u64` |
| 6 | rdev | rdev `u64` |
| 7 | symlink target | target blob; valid only for kind `3` |
| 8 | allocated size | allocated size `u64` |
| 9 | names | owner name UTF-8, group name UTF-8 (each at most 256 bytes) |
| 10 | flags | flags `u32`: bit 0 immutable, bit 1 append-only, bit 2 hidden |

A presence bit above 10 is a protocol error. A symlink target on an entry whose
kind is not `3` is a protocol error. A `DirPage` entry is a name blob (one
relative component, raw bytes, at most 1 MiB) followed by an attrs record.

### The attr mask

`Open`, `Stat` and `ReadDir` each carry an `attr mask u32` naming the optional
blocks the client wants in the reply, using the same bit numbering as the
presence bitmap. A mask of `0` requests the fixed part only.

The mask and the bitmap have deliberately opposite rules for unknown bits:

- **Unknown mask bits are ignored**, like capability bits. A newer client may
  ask an older server for a block it has never heard of; the server answers
  without that block instead of failing the request.
- **Unknown presence bits are rejected**, because a decoder cannot skip a block
  whose length it does not know.

A response's presence bitmap is a subset of the request's mask and may be a
strict subset: a block the filesystem does not keep (`btime`), or one whose
lookup the server declines (`names`), is simply absent. A client must render
correctly with every optional block missing. The symlink target is bit 7 like
any other block, so an entry of kind `3` carries its target only when the mask
asked for it.

### v3 field encoding and bounds

- Paths, export names and entry names are raw byte strings of at most 1 MiB.
  `Mount` with an empty export names the server's `--server` root. Paths are
  relative to the export root; intermediate symlink components are rejected as
  a traversal escape, as in v2.
- `Features` carries the sender's optional-feature bitmap; the negotiated set
  is the intersection and **unknown bits are ignored**, so peers of different
  ages agree. Defined bits: `1 LOCKS`, `2 LEASES`, `4 SHARE_MODES`, `8 NOTIFY`,
  `16 NOTIFY_POLLING`, `32 XATTR`, `64 SPARSE`, `128 COMPOUND`,
  `256 STAGE_RESUME`, `512 ACCESS`, `1024 OWNER_NAMES`. Phase 1 defines the
  bits and the exchange; the message groups they gate are later phases. A
  client never sends a type whose feature bit the server did not advertise.
- Access is `0` read-write or `1` read-only. Normalization is `0` none, `1`
  NFC, `2` NFD.
- `MountInfo.effective_writable` is the single source of truth for whether the
  session may write. `reason` is required (non-empty) when it is false and must
  be empty when it is true; `access=1` with `effective_writable=true` is a
  protocol error. `access` is what the export grants and `effective_writable`
  is what this session got, so a client asking for `ro` on a `rw` export sees
  `access=0` with `effective_writable=false` and a reason saying so.
  `supports` bits (`1 XATTRS`, `2 SYMLINKS`, `4 HARDLINKS`, `8 LOCKS`,
  `16 LEASES`, `32 NOTIFY`, `64 NOTIFY_POLLING`, `128 SPARSE`,
  `256 CASE_INSENSITIVE`, `512 NORMALIZATION_INSENSITIVE`) are set only when
  the filesystem can do the thing **and** this server exposes it, so a bit is a
  promise a client may act on rather than a description of the volume. Unknown
  bits are preserved. `max read` and `max write` are in `1..=8 MiB` and bound
  `Read.length` and `Write.data`. Cache hints of `0` mean "no hint".
- `normalization` is the form the filesystem *applies* to a name it is given.
  Whether it can tell two canonically-equivalent forms apart is the separate
  `NORMALIZATION_INSENSITIVE` bit, because a filesystem may preserve what it is
  given and still fold the two for comparison, as APFS does. Neither field
  implies the other, and a client that writes names needs the bit.
- `Open.flags` bits: `1 READ`, `2 WRITE`, `4 CREATE`, `8 EXCL`, `16 TRUNC`,
  `32 APPEND`, `64 NOFOLLOW`, `128 DIRECTORY`. Any other bit is a protocol
  error, as is: no `READ`, `WRITE` or `DIRECTORY`; `CREATE`, `EXCL`, `TRUNC` or
  `APPEND` without `WRITE`; `EXCL` without `CREATE`; `DIRECTORY` with any write
  flag. `mode` is at most `0o7777` and must be `0` unless `CREATE` is set. A
  write-class open against a mount whose `effective_writable` is false is
  refused with `Error` code `3 EROFS` before any filesystem call.
- Handles are `u64`, session-scoped, and never reused within a session. `0` is
  never a handle. They are not unique across sessions and need not be: a handle
  means nothing to a session that did not open it, and a session that resumes
  (E3-S2) keeps its own table. Requests on one handle are applied in send
  order; requests on different handles or paths have no ordering guarantee and
  the server may answer them out of order (responses correlate by related
  request ID).
- "Applied in send order" binds only where an operation can observe another.
  `Read`, `ReadDir` and a handle `Stat` do not mutate and so cannot observe
  each other: a server may run several of them on one handle at once, which is
  what lets a client keep a window of reads outstanding on a single file
  instead of paying a round trip per chunk. `Write`, `Flush` and `Close` take
  the handle to themselves — they wait for the requests sent before them to
  finish, and the requests sent after them wait in turn. The queue stays FIFO,
  so a `Write` behind a burst of reads is not starved by later ones.
- `Open` answers `Opened` with the handle and the target's attributes, so an
  open needs no follow-up `Stat`. A directory must be opened with `DIRECTORY`,
  and a directory opened without it is `EISDIR` rather than a handle no read or
  write could use; `DIRECTORY` on a non-directory is `ENOTDIR`. With
  `NOFOLLOW`, a final symlink is `ELOOP`. A path that leaves the export is
  refused before any filesystem call: `EINVAL` for one the wire format rejects
  (absolute, empty component, `.`, `..`, NUL) and `EACCES` for a symlink in a
  parent component.
- A server bounds the handles one session may hold; past the bound `Open` is
  `ELIMIT`. `Close` releases the handle and any lock or lease held through it;
  closing a handle that is not open is `EBADF` for that request and never a
  session error. Every handle is released when the session ends.
- A server bounds the accepted-but-unanswered requests on one session. Past that
  bound a request is answered with `Error` code `24 ELIMIT` and is not executed;
  the session itself is unaffected and the client may retry once a response has
  freed a slot. A server must not stall its reader to apply the bound, because
  `Keepalive` and `Cancel` have to stay answerable while the pool is busy — those
  two and the `Features` exchange are answered without waiting for any
  filesystem work. Negotiated credit-based flow control is a later revision.
- `Cancel` on a request that has not started yet is that request's terminal
  response (`ECANCELED`); on one already executing it is advisory, and the
  request's own terminal response is the only answer the client receives, so a
  request is never answered twice.
- `Read` requires a handle opened with `READ`; one opened write-only is
  `EACCES` and a directory handle is `EISDIR` (use `ReadDir`). `Read.length` is
  `1..=max read`, and a length above the `max read` the mount advertised is
  `EINVAL` even though the envelope could carry it. `ReadData.data` is at most
  8 MiB; a short read is legal only with `eof=true`, and `eof` means the server
  reached end of file rather than that it chose to return less — a short read
  for any other reason is not permitted. `ReadData.offset` echoes the request,
  so a client matching responses needs only the id.
- `Write` requires a handle opened with `WRITE`; one opened read-only is
  `EACCES` and a directory handle is `EISDIR`. `Write.data` is
  `1..=max write`, and data longer than the `max write` the mount advertised is
  `EINVAL`. When a digest is present it is BLAKE3 of `data`, verified before
  the first byte reaches the file; a `Write` whose digest does not match is
  refused with code `23 EINTEGRITY` and nothing is written.
- A `Write` that fails part-way has already put some bytes in the file. The
  server answers with the error rather than a short `WriteAck`, because a short
  acknowledgement cannot say which bytes landed; a client that needs to know
  re-reads the range.
- An `APPEND` handle writes at the end of the file and ignores `Write.offset`.
  `WriteAck` carries no offset of its own, so the offset an append landed at is
  `new size - bytes written`; under the handle's exclusive domain that is
  exact.
- `WriteAck.stable` is true only when the bytes are already durable (export
  configured `sync`); otherwise `Flush` is the durability barrier and answers
  `Done`. A server with no `sync` option always reports `false`, which is
  honest rather than optimistic: a client that needs durability must `Flush`.
  `Flush` on a handle that was only read is not an error — there is simply
  nothing outstanding — but on a directory handle it is `EISDIR`, because a
  directory handle buffers nothing. `Close` answers `Done` and releases the
  handle.
- `Stat` on a file handle answers from the descriptor, so it describes the file
  this session opened even if the name has since been reused; on a directory
  handle, which holds no descriptor, it restats the path. `Stat.target` is `0`
  path or `1` handle. For a path target the handle field
  must be `0`; for a handle target the path must be empty; otherwise the
  payload is a protocol error. `follow` selects `stat` over `lstat` and is
  ignored for a handle target. The `attr mask` follows the rules above; asking
  for `names` requires the `OWNER_NAMES` feature and is otherwise answered
  without that block.
- `ReadDir` requires a handle opened with `DIRECTORY`; a file handle is
  `ENOTDIR`. Its `attr mask` applies to every entry in the page, which is what
  makes one round trip a readdirplus: the server still stats each entry, since
  the fixed part of `Attrs` cannot be answered from the directory alone, but
  the client pays one round trip rather than one per entry. `max entries` is
  `1..=65,536`; `count` is capped at 65,536 before reserving memory. `.` and
  `..` are never returned.
- Cursor `0` starts a fresh snapshot of the directory's names; a non-final page
  returns the cursor for the next page, and a cursor past the end of that
  snapshot is `EINVAL`. Paging is a position within the snapshot, so a page
  costs the size of the page and not the size of the offset — re-reading the
  directory per page and skipping forward makes a listing quadratic in its own
  length. An entry created or removed while a listing is in progress may or may
  not appear, but no entry appears twice and an entry present throughout is
  never missed; a name that has disappeared by the time its page is built is
  left out rather than failing the page.
- `FsInfo.read_only` describes the filesystem, distinct from the export's
  access: a writable filesystem behind a `ro` export reports `false` here and
  `ro` in `MountInfo`. `fs type` is UTF-8 of at most 256 bytes, and empty means
  the server does not know it. Inode counts of `0` mean unknown. A server
  reports unknown rather than guessing, so a client must be able to display a
  mount whose inode counts and filesystem name it never learns.
- `free bytes` counts every free byte; `available bytes` counts what this
  identity may actually use, which on most Unix filesystems is smaller because
  of the root-only reserve. A capacity display wants `available`. `FsInfo`
  repeats `max name len`, `case sensitive` and `normalization` from `MountInfo`
  and must agree with it: they describe the same filesystem and a server
  answers both from one probe.
- `Error.code` is frozen: `1 ENOENT`, `2 EACCES`, `3 EROFS`, `4 EEXIST`,
  `5 ENOTEMPTY`, `6 EISDIR`, `7 ENOTDIR`, `8 EXDEV`, `9 ESTALE`, `10 ENOSPC`,
  `11 EDQUOT`, `12 ENAMETOOLONG`, `13 ELOOP`, `14 EBUSY`, `15 EWOULDBLOCK`,
  `16 ETIMEDOUT`, `17 ECANCELED`, `18 EBADF`, `19 EINVAL`, `20 EIO`,
  `21 EOPNOTSUPP`, `22 EILSEQ`, `23 EINTEGRITY`, `24 ELIMIT`, `25 ECHANGED`,
  `26 ELEASEBROKEN`. Any other value is a protocol error. `platform errno` is
  `0` when unavailable; the message is UTF-8 of at most 64 KiB.
- UTF-8 text fields (`reason`, `options`, error messages) are capped at 64 KiB.
  Booleans are exactly `0` or `1`.
- The 16 MiB encoded and decoded payload caps, checked length arithmetic,
  reserved-field rules and message-ID uniqueness apply unchanged. Phase 1
  frames are uncompressed; the envelope compression flag is reserved for a
  later phase and a compressed v3 frame is rejected today.

### v3 session handling

- After the selected-version boundary a v3 session accepts only the table
  above plus types 18–20. A v1 or v2 frame after a v3 selection, or a v3 frame
  after a v1 or v2 selection, is a protocol error.
- The client sends `Features` immediately after selection and `Mount` after
  `FeaturesAck`; every other request requires a completed `Mount` and is
  answered `EINVAL` before one. A session mounts once; a second `Mount` is
  `EINVAL`. Because the mount answers whether the session may write at all, a
  server answers it before it begins serving anything else rather than
  concurrently with it.
- On a mount whose `effective_writable` is false, every write-class request --
  `Write`, and `Open` carrying `WRITE`, `CREATE`, `EXCL`, `TRUNC` or `APPEND` --
  is refused with `EROFS` before the filesystem is touched, and the refusal
  carries the mount's own `reason`, so a client shows one explanation
  everywhere. Such a refusal is an expected answer, not a server fault.
- Errors are per-request and do not terminate the session unless the error is a
  framing, bounds, duplicate-ID or other protocol error.
- Byte-exact vectors live in `protocol-v3-vectors/`; the codec is
  `xsync-core::protocol_v3`.
