# Protocol v2 Conformance Vectors

This directory is the canonical fixture corpus for the initial v2 payload
table in `protocol.md`. The corpus is intentionally line-oriented rather than
language-specific: a consumer reads tab-separated fields and decodes the hex
payload without depending on Rust enum names or JSON libraries.

`payload-v1.tsv` contains one vector per line:

```text
id<TAB>direction<TAB>type<TAB>payload-hex<TAB>decoded-meaning
```

The decoded meaning is a compact, human-readable description of the fields in
wire order. Payload bytes are little-endian as specified by `protocol.md`.
Lines beginning with `#` are comments. Vector revisions are protocol changes;
the consuming f2 copy must record the source revision when it is imported.

Types 36–41 (`CAP_BROWSE_META`) were added for Kestrel XS-B2. f2's copied
fixture has not been updated; that is a named release blocker, not a wire
incompatibility. An f2 build without those types does not advertise the bit.
