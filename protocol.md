# xsync Protocol v1

This document is the compatibility contract for the v1 remote protocol. The
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
| 8 | 4 | protocol version, exactly `1` |
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
| 4 | FileSegment | file ID `u64`, offset `u64`, data bytes |
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

This table is the v2 freeze for Stories 9.1, 9.2, 11.1, 12.1, and 12.2. Additional
session controls receive new type assignments in a later protocol revision or
an explicitly amended v2 table; they must not reuse these numbers.

Malformed compression, a compressed output larger than the declared bounded
length, or a declared decompressed length over 16 MiB is rejected before any
filesystem publication. Protocol decoding does not write destination files;
the sink may consume only fully decoded and separately verified operations.
