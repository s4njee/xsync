# xsync Protocol v1

This document is the compatibility contract for the v1 remote protocol. The
Rust types in `xsync-core::protocol` implement this layout; Rust enum layout or
the choice of serialization library is not observable on the wire.

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
of received IDs and rejects duplicates before dispatch. A session may not
exceed 1,048,576 tracked IDs.

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

Malformed compression, a compressed output larger than the declared bounded
length, or a declared decompressed length over 16 MiB is rejected before any
filesystem publication. Protocol decoding does not write destination files;
the sink may consume only fully decoded and separately verified operations.
