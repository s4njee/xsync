#!/usr/bin/env python3
"""Generate protocol-v3-vectors/payload-v1.tsv from the protocol.md v3 layout.

This is deliberately an independent implementation of the field-by-field
contract in `protocol.md` ("v3 message table"), not a wrapper around the Rust
codec. The Rust conformance test decodes these bytes and re-encodes them
byte-exact; disagreement means one of the two implementations has drifted from
the document, and the document wins.
"""

from __future__ import annotations

import struct
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "protocol-v3-vectors" / "payload-v1.tsv"


def u8(v: int) -> bytes:
    return struct.pack("<B", v)


def u16(v: int) -> bytes:
    return struct.pack("<H", v)


def u32(v: int) -> bytes:
    return struct.pack("<I", v)


def u64(v: int) -> bytes:
    return struct.pack("<Q", v)


def i32(v: int) -> bytes:
    return struct.pack("<i", v)


def i64(v: int) -> bytes:
    return struct.pack("<q", v)


def boolean(v: bool) -> bytes:
    return u8(1 if v else 0)


def blob(v: bytes) -> bytes:
    return u32(len(v)) + v


def attrs(
    kind: int,
    mode: int,
    size: int,
    mtime_ns: int,
    cookie: bytes,
    *,
    presence: int = 0,
    owner: tuple[int, int] | None = None,
    target: bytes | None = None,
) -> bytes:
    """The attrs record: presence bitmap, fixed part, then optional blocks in bit order.

    A request's attr_mask uses this same bit numbering; see protocol.md.
    """
    assert len(cookie) == 16
    out = u32(presence) + u8(kind) + u32(mode) + u64(size) + i64(mtime_ns) + cookie
    if presence & 0x01:
        assert owner is not None
        out += u32(owner[0]) + u32(owner[1])
    if presence & 0x80:
        assert target is not None
        out += blob(target)
    return out


COOKIE = lambda byte: bytes([byte]) * 16  # noqa: E731

VALID: list[tuple[str, str, int, bytes, str]] = [
    ("cancel-shared-with-v2", "request", 18, u64(7), "related_id=7"),
    ("features", "request", 42, u64(3), "features=3 (LOCKS|LEASES)"),
    ("features-ack", "response", 43, u64(1) + u64(1), "related_id=1, features=1 (LOCKS)"),
    ("mount-root-rw", "request", 50, blob(b"") + u8(0), "export=[] (server root), requested_access=rw"),
    (
        "mount-info-ro-squashed",
        "response",
        51,
        u64(2)
        + blob(b"media")
        + u8(1)
        + boolean(False)
        + blob(b"export is ro")
        + blob(b"ro,root_squash")
        + boolean(True)
        + u8(0)
        + u32(255)
        + u32(4096)
        + u64(2)
        + u32(1 << 20)
        + u32(1 << 20)
        + u32(0)
        + u32(0),
        'related_id=2, export="media", access=ro, effective_writable=false, reason="export is ro", '
        'options="ro,root_squash", case_sensitive=true, normalization=none, max_name_len=255, '
        "max_path_len=4096, supports=2 (SYMLINKS), max_read=1048576, max_write=1048576, "
        "attr_cache_ms=0, dir_cache_ms=0",
    ),
    ("open-read-nofollow", "request", 56, blob(b"a") + u32(0x41) + u32(0) + u32(0x20), "path=[61], flags=0x41 (READ|NOFOLLOW), mode=0, attr_mask=0x20 (IDENTITY)"),
    (
        "opened-minimal-attrs",
        "response",
        57,
        u64(3) + u64(10) + attrs(1, 0o644, 4, -2, COOKIE(0x11)),
        "related_id=3, handle=10, attrs(presence=0, kind=1, mode=420, size=4, mtime_ns=-2, cookie=[11;16])",
    ),
    ("close", "request", 58, u64(10), "handle=10"),
    ("read-with-digest", "request", 59, u64(10) + u64(4096) + u32(65536) + boolean(True), "handle=10, offset=4096, length=65536, want_digest=true"),
    (
        "read-data-eof-digest",
        "response",
        60,
        u64(4) + u64(4096) + boolean(True) + boolean(True) + bytes([0xAB]) * 32 + blob(b"hi"),
        'related_id=4, offset=4096, eof=true, digest_present=true, digest=[ab;32], data="hi"',
    ),
    ("write-plain", "request", 61, u64(10) + u64(0) + boolean(False) + blob(b"hi"), 'handle=10, offset=0, digest_present=false, data="hi"'),
    (
        "write-ack",
        "response",
        62,
        u64(5) + u32(2) + u64(2) + boolean(False) + COOKIE(0x22),
        "related_id=5, bytes_written=2, new_size=2, stable=false, cookie=[22;16]",
    ),
    ("flush", "request", 63, u64(10), "handle=10"),
    ("stat-path-follow", "request", 80, u8(0) + blob(b"a") + u64(0) + boolean(True) + u32(0x201), "target=path, path=[61], handle=0, follow=true, attr_mask=0x201 (OWNER|NAMES)"),
    (
        "attrs-symlink-with-owner",
        "response",
        81,
        u64(6) + attrs(3, 0o777, 1, 0, COOKIE(0x33), presence=0x81, owner=(1000, 1000), target=b"b"),
        "related_id=6, attrs(presence=0x81 (OWNER|SYMLINK_TARGET), kind=3, mode=511, size=1, mtime_ns=0, "
        "cookie=[33;16], uid=1000, gid=1000, target=[62])",
    ),
    ("readdir", "request", 82, u64(11) + u64(0) + u32(256) + u32(0x7FF), "handle=11, cursor=0, max_entries=256, attr_mask=0x7ff (every known block)"),
    ("readdir-forward-compatible-mask", "request", 82, u64(11) + u64(0) + u32(256) + u32(0x800), "handle=11, cursor=0, max_entries=256, attr_mask=0x800 (a bit this version does not define; ignored, not rejected)"),
    (
        "dir-page-one-entry",
        "response",
        83,
        u64(7) + u64(0) + boolean(True) + u32(1) + blob(bytes([0xFF, ord("x")])) + attrs(2, 0o755, 0, 0, COOKIE(0x44)),
        "related_id=7, cursor=0, final=true, count=1, entry(name=[ff78], attrs(presence=0, kind=2, mode=493, "
        "size=0, mtime_ns=0, cookie=[44;16]))",
    ),
    ("statfs", "request", 84, b"", "no fields"),
    (
        "fs-info",
        "response",
        85,
        u64(8) + u32(4096) + u64(1000) + u64(500) + u64(400) + u64(10) + u64(5) + blob(b"apfs") + u32(255) + boolean(False) + u8(1) + boolean(False),
        'related_id=8, block_size=4096, total_bytes=1000, free_bytes=500, available_bytes=400, total_inodes=10, '
        'free_inodes=5, fs_type="apfs", max_name_len=255, case_sensitive=false, normalization=nfc, read_only=false',
    ),
    # E5-S4 mutations (types 86-95).
    ("rename-noreplace", "request", 86, blob(b"a") + blob(b"b") + u8(0) + u32(0), "source=[61], destination=[62], mode=0 (NOREPLACE), attr_mask=0"),
    ("rename-exchange", "request", 86, blob(b"a") + blob(b"b") + u8(2) + u32(0x201), "source=[61], destination=[62], mode=2 (EXCHANGE), attr_mask=0x201 (OWNER|NAMES)"),
    ("unlink", "request", 87, blob(b"a"), "path=[61]"),
    ("rmdir", "request", 88, blob(b"d"), "path=[64]"),
    ("mkdir", "request", 89, blob(b"d") + u32(0o755) + u32(0), "path=[64], mode=493, attr_mask=0"),
    ("symlink-escaping-target", "request", 90, blob(b"../outside") + blob(b"l") + u32(0x80), "target=[2e2e2f6f757473696465] (stored verbatim, never resolved), path=[6c], attr_mask=0x80 (SYMLINK_TARGET)"),
    ("link", "request", 91, blob(b"a") + blob(b"b") + u32(0), "existing=[61], path=[62], attr_mask=0"),
    (
        "chown-uid-only",
        "request",
        92,
        u8(0) + blob(b"a") + u64(0) + boolean(True) + u32(1000) + boolean(False) + u32(0) + boolean(False) + u32(0),
        "target=path, path=[61], handle=0, uid_present=true, uid=1000, gid_present=false, gid=0, follow=false, attr_mask=0",
    ),
    (
        "set-times-omit-and-set",
        "request",
        93,
        u8(1) + blob(b"") + u64(4) + u8(0) + i64(0) + u32(0) + u8(2) + i64(-1) + u32(999_999_999) + boolean(True) + u32(0),
        "target=handle, path=[], handle=4, atime=OMIT, mtime=SET(-1s, 999999999ns), follow=true, attr_mask=0; "
        "every TimeChange tag is the same width so the layout never depends on the value",
    ),
    (
        "set-times-now",
        "request",
        93,
        u8(0) + blob(b"a") + u64(0) + u8(1) + i64(0) + u32(0) + u8(1) + i64(0) + u32(0) + boolean(True) + u32(0),
        "target=path, path=[61], handle=0, atime=NOW, mtime=NOW (resolved by the server, not the client), follow=true, attr_mask=0",
    ),
    (
        "set-permissions",
        "request",
        94,
        u8(0) + blob(b"a") + u64(0) + u32(0o640) + boolean(True) + u32(0),
        "target=path, path=[61], handle=0, mode=416, follow=true, attr_mask=0",
    ),
    (
        "mutated",
        "response",
        95,
        u64(12) + attrs(2, 0o755, 0, 7, COOKIE(0x55)),
        "related_id=12, attrs(presence=0, kind=2, mode=493, size=0, mtime_ns=7, cookie=[55;16])",
    ),
    ("error-erofs", "response", 121, u64(9) + u16(3) + i32(30) + blob(b"read-only"), 'related_id=9, code=3 (EROFS), platform_errno=30, message="read-only"'),
    ("done", "response", 122, u64(9), "related_id=9"),
]

MALFORMED: list[tuple[str, int, bytes, str]] = [
    ("unknown-type", 200, b"", "invalid message type"),
    ("open-unknown-flag", 56, blob(b"a") + u32(0x100) + u32(0) + u32(0), "unknown open flag"),
    ("open-trunc-without-write", 56, blob(b"a") + u32(0x11) + u32(0) + u32(0), "open flags inconsistent"),
    ("read-zero-length", 59, u64(10) + u64(0) + u32(0) + boolean(False), "read length out of range"),
    ("attrs-unknown-presence-bit", 81, u64(6) + attrs(1, 0, 0, 0, COOKIE(0), presence=0) [:0] + u32(1 << 11) + u8(1) + u32(0) + u64(0) + i64(0) + COOKIE(0), "unknown attrs presence bit"),
    ("attrs-target-on-regular-file", 81, u64(6) + attrs(1, 0, 0, 0, COOKIE(0), presence=0x80, target=b"b"), "symlink target on non-symlink"),
    (
        "mount-info-writable-with-reason",
        51,
        u64(2) + blob(b"") + u8(0) + boolean(True) + blob(b"x") + blob(b"") + boolean(True) + u8(0) + u32(255) + u32(4096) + u64(0) + u32(1 << 20) + u32(1 << 20) + u32(0) + u32(0),
        "writability inconsistent with reason",
    ),
    ("error-code-zero", 121, u64(9) + u16(0) + i32(0) + blob(b""), "invalid error code"),
    ("trailing-byte", 122, u64(9) + u8(0), "trailing payload byte"),
    ("truncated-attrs", 57, u64(3) + u64(10) + u32(0) + u8(1), "truncated attrs"),
    ("stat-handle-with-path", 80, u8(1) + blob(b"a") + u64(10) + boolean(False) + u32(0), "stat target inconsistent"),
    ("readdir-zero-entries", 82, u64(11) + u64(0) + u32(0) + u32(0), "page size out of range"),
    ("rename-unknown-mode", 86, blob(b"a") + blob(b"b") + u8(3) + u32(0), "invalid rename mode"),
    (
        "set-times-unknown-tag",
        93,
        u8(0) + blob(b"a") + u64(0) + u8(3) + i64(0) + u32(0) + u8(0) + i64(0) + u32(0) + boolean(True) + u32(0),
        "invalid time change tag",
    ),
    (
        "set-times-nanos-out-of-range",
        93,
        u8(0) + blob(b"a") + u64(0) + u8(2) + i64(0) + u32(1_000_000_000) + u8(0) + i64(0) + u32(0) + boolean(True) + u32(0),
        "nanoseconds out of range",
    ),
    (
        "chown-handle-with-path",
        92,
        u8(1) + blob(b"a") + u64(4) + boolean(False) + u32(0) + boolean(False) + u32(0) + boolean(False) + u32(0),
        "stat target inconsistent",
    ),
]


def main() -> None:
    lines = [
        "# id\tdirection\ttype\tpayload hex\tdecoded meaning",
        "# Protocol v3, Phase 1 table. Generated by scripts/generate-v3-vectors.py from the protocol.md layout;",
        "# the Rust codec is tested against these bytes, never the other way round.",
        "# Shared v2 control type 18 appears once to pin that its layout is unchanged in a v3 session.",
    ]
    for ident, direction, mtype, payload, meaning in VALID:
        lines.append(f"{ident}\t{direction}\t{mtype}\t{payload.hex()}\t{meaning}")
    lines.append("")
    lines.append("# Deliberately malformed payloads. The final field names the fail-closed rule.")
    for ident, mtype, payload, rule in MALFORMED:
        lines.append(f"{ident}\tmalformed\t{mtype}\t{payload.hex()}\t{rule}")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(VALID)} valid and {len(MALFORMED)} malformed vectors to {OUT.relative_to(OUT.parents[1])}")


if __name__ == "__main__":
    main()
