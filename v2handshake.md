# xsync Protocol v2 Handshake Contract

Status: implemented contract for Story 9.1. This document defines negotiation
only. The v2 message table and envelope payloads belong to Story 9.2.

## Goals

- A v2 client can use a v1 sync session when the peer has no v2 support.
- A v1 client never receives a v2 frame.
- The selected version is decided exactly once before `SessionConfig` or any
  data frame is sent.
- Browse capability is reported separately from sync compatibility.
- Existing v1 errors remain unchanged when negotiation is impossible.

## Opening Frame

The opening handshake is always sent in a v1 envelope:

| Field | Value |
|---|---|
| magic | `xsn1` |
| envelope version | `1` |
| message type | `Handshake` (`1`) |
| payload | The frozen v1 `Handshake` payload |

No v2-only field is appended to the v1 handshake. A v1 decoder therefore
accepts the opening frame without a compatibility exception. The capability
bitmap is the only negotiation extension point.

A v2 implementation MUST support receiving and sending this opening frame,
even when its eventual session uses v2 frames.

## Capability Bits

Capability bits are advertised independently by each endpoint. Unknown bits are
ignored as specified by `protocol.md`.

| Bit | Name | Meaning |
|---:|---|---|
| 0 | `CAP_DATA_ONLY` | Existing v1 data-only receiver behavior |
| 1 | `CAP_ZSTD` | Existing zstd payload support |
| 2 | `CAP_BROWSE_V2` | Endpoint can use the v2 browse message set |
| 3 | `CAP_VERSION_NEGOTIATION` | Endpoint understands this handshake contract |
| 6 | `CAP_BROWSE_META` | Endpoint understands browse types 36–41 (chmod, mtime, readlink) |

The v1 implementation advertises `CAP_VERSION_NEGOTIATION` after this change,
but does not advertise `CAP_BROWSE_V2`. A v2 implementation advertises both
bits 2 and 3 when it implements the browse surface.

`CAP_VERSION_NEGOTIATION` alone never selects v2. It means only that the peer
understands the deterministic selection rule.

## Version Selection

After both `Handshake` messages have been received and the server's handshake
acknowledged, both endpoints calculate the same result:

```text
v2 = local has CAP_VERSION_NEGOTIATION and CAP_BROWSE_V2
  && remote has CAP_VERSION_NEGOTIATION and CAP_BROWSE_V2

selected_version = 2 if v2 else 1
```

The selected version is committed before the next request. There is no retry,
probe, or mid-session fallback.

### v2 client to v1 server

The v2 client sends a v1 opening frame. The v1 server responds with its v1
handshake and does not advertise `CAP_BROWSE_V2`. Both endpoints select v1.
The client sends ordinary v1 `SessionConfig` and transfer frames. Browse is
reported unavailable, but push and pull continue normally.

### v1 client to v2 server

The v1 client does not advertise `CAP_BROWSE_V2`. Both endpoints select v1.
The v2 server MUST send only v1 frames for the session. The v1 client does not
need to know that the server supports v2.

### v2 client to v2 server

Both endpoints advertise the two negotiation bits and select v2. The next
frame after the v1 handshake acknowledgement uses the v2 envelope and the v2
message table. Story 9.2 defines those message types and their bounds.

## Ordering

The handshake sequence is:

1. Client sends v1-envelope `Handshake`.
2. Server sends v1-envelope `Handshake`.
3. Server acknowledges the client's handshake using a v1-envelope `Ack`.
4. Both endpoints calculate `selected_version` from the two capability sets.
5. The client sends `SessionConfig` in the selected version.
6. Transfer or browse frames follow in the selected version only.

The client MUST expose the selected version, remote capability bitmap, and
browse availability before step 5 is sent. The existing
`protocol-negotiated` event is the CLI/embedding boundary for this state.

## Failure and Downgrade Rules

- Downgrade is allowed only at step 4, before `SessionConfig`.
- A v1 selection must never be inferred from a v2 decode failure.
- A v2 session that receives a malformed or unsupported v2 frame fails closed;
  it must not restart the handshake or retry as v1.
- A peer that cannot decode the v1 opening envelope remains genuinely
  incompatible. The existing error is preserved exactly:
  `xsync version mismatch: local vX / remote vY`.
- Unknown capability bits do not fail negotiation.
- Capability presence does not authorize a message type. The selected version
  still controls the complete message and envelope grammar.

## Observability

The negotiation event must contain:

- `selected_version`: `1` or `2` (the implementation's existing `wire_version`
  transport field is populated from this value);
- `remote_capabilities`: the raw remote bitmap;
- `common_capabilities`: the intersection of local and remote known bits;
- `browse_available`: true only when v2 browse was selected and implemented.

The event is emitted before the first selected-version frame. JSON consumers
must be able to disable browse controls without parsing human output.

## Test Matrix

The conformance suite must cover:

- v2-capable client plus v1 server: push and pull complete using v1 frames;
- v1 client plus v2 server: session completes and no v2 frame is emitted;
- v2 client plus v2 server: selection is v2 before `SessionConfig`;
- one-sided capability advertisement: selection is v1;
- unknown capability bits: ignored without changing selection;
- a v2 frame after a v1 selection: rejected as a protocol error;
- a v1 frame after a v2 selection: rejected as a protocol error;
- incompatible opening envelope: exact existing version-mismatch wording;
- no fallback after a malformed or failed mid-session frame.

Byte-exact v2 message vectors are deferred to Story 9.3 after Story 9.2 freezes
the message table.
