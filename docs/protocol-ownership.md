# Protocol ownership and change process

This is the change contract for the wire format shared by xsync and f2. The
xsync repository owns the canonical `protocol.md`, `v2handshake.md`, and
`protocol-v2-vectors/` files. The f2 repository is a consuming implementation;
it reviews protocol changes and carries a copied vector fixture with the xsync
source revision recorded in its header.

## Who may change the contract

Anyone may propose a protocol change, but only a reviewed change merged in the
xsync repository changes the canonical wire contract. A change to `protocol.md`,
`v2handshake.md`, a type assignment, or a canonical vector must include:

1. the field-by-field specification and bounds;
2. Rust codec tests, including malformed-input coverage;
3. an updated compatibility matrix; and
4. the copied-vector update and consumer test change in f2, or an explicit
   blocker naming the missing consumer change.

The f2 maintainers receive review before the xsync change lands. xsync owns
the final merge decision because there must be one canonical assignment source,
but a protocol change is not release-ready until both implementations have
either landed it or documented why the older implementation safely degrades.

## Type bytes and versions

Message type bytes are assigned once. A byte is never reused, never given a new
meaning, and never made conditional on a peer's private implementation. A
payload change that alters field order, bounds, enum meanings, or required
fields is a breaking change even if it keeps the same type byte.

Adding a message type is not compatible with v1 because v1 is fail-closed. A
new message therefore requires a new protocol version (or a previously frozen
capability-gated assignment in that version). A breaking change to an existing
message requires a version bump and a new compatibility-matrix row/column;
it must not be smuggled in as an optional field.

## Landing coordinated changes

When both projects need a change, the sequence is:

1. xsync lands the specification, canonical vectors, codec tests, and a
   compatibility entry for the old peer;
2. f2 updates its copied vectors and implementation, recording the xsync
   revision;
3. both projects run their local suites plus the joint smoke command; and
4. release notes identify the first versions containing the change.

If the two implementations cannot land together, the wire-compatible half may
land only when the older peer has a defined outcome in the matrix (working,
reduced capability, or the exact refusal). A half-implemented feature must not
be advertised by a capability bit. The release is blocked when no such safe
intersection exists.

## Review checklist

Every protocol PR records the assigned version/type bytes, raw-byte and UTF-8
rules, all bounds, response correlation rules, downgrade behavior, vector
revision, matrix result, and the joint-smoke result or its named blocker.
