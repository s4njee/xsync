# Protocol v3 Conformance Vectors

This directory is the canonical fixture corpus for the Phase 1 v3 filesystem
payload table in `protocol.md` ("v3 message table"). It has the same shape as
`protocol-v2-vectors/`: a consumer reads tab-separated fields and decodes the
hex payload without depending on Rust enum names or JSON libraries.

`payload-v1.tsv` contains one vector per line:

```text
id<TAB>direction<TAB>type<TAB>payload-hex<TAB>decoded-meaning
```

The decoded meaning is a compact, human-readable description of the fields in
wire order. Payload bytes are little-endian as specified by `protocol.md`.
Lines beginning with `#` are comments. Vector revisions are protocol changes
and follow `docs/protocol-ownership.md`.

The corpus covers every Phase 1 type once in its valid form, plus one shared v2
control type (18) to pin that its layout is unchanged in a v3 session, and one
malformed payload per fail-closed rule the codec enforces: unknown type, unknown
open flag, inconsistent open flags, out-of-range read length, unknown `Attrs`
presence bit, a symlink target on a non-symlink, writability that disagrees with
its reason, an error code outside the frozen table, a trailing byte, a truncated
`Attrs`, an inconsistent `Stat` target, and an out-of-range `ReadDir` page size.

No consumer other than xsync has imported this corpus yet. Excalibur
(`~/projects/excalibur`) is the first planned consumer; f2 does not implement
v3 and never advertises `CAP_FS_V3`.
