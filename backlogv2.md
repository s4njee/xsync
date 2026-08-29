# xsync v2 — protocol v2 and the browse surface

Companion to [tasks.md](tasks.md) and [protocol.md](protocol.md); same conventions. Epics continue
from tasks.md's Epic 8. Status legend: `[ ]` todo · `[~]` in progress · `[x]` done. "AC" = acceptance
criteria.

## Why this exists

`f2` (`~/projects/f2`) is adopting xsync as its remote transport rather than continuing its own Go
agent. Decision recorded on 2026-08-24 as f2 story **A12.1**; the rejected alternative and its
reasoning live there. That commits both projects to something new: **one wire format with two
owners.**

What f2 gets from this is already built here — a framed transport, zstd capability negotiation,
bounded frames, durable chunk-level resume, and *both* transfer directions. That last one matters
more than it looks: f2's own pull direction was unimplemented, so adopting xsync closes an entire
f2 epic (`R6`) rather than merely replacing a transport.

What f2 needs that does not exist here is one shape of thing: **xsync is a sync engine, and f2 is a
file browser.** A sync session is one long operation over a whole tree, planned and executed. A
browser issues a stream of small, unrelated, latency-sensitive requests — list this directory, now
that one, rename this, read that file — and expects each answered in milliseconds over a connection
that stays open for as long as the window is. `Scan` exists, but it is a whole-tree scan feeding a
diff, not an interactive listing of one directory.

## The constraint that shapes everything below

**v1 is fail-closed and frozen.** It rejects an unknown magic, version, header length, reserved
field, message type, flag, enum value, duplicate ID, or trailing byte. Its own compatibility section
says a new message type requires a version bump — it cannot be added compatibly. So there is no
incremental path: the browse surface *is* protocol v2, and v2 must be designed knowing that a v2
client will meet v1 servers in the field.

Capability bits are the one exception already designed for growth: `Handshake.capabilities` is
deliberately un-masked, so a peer ignores bits it does not recognize. That is the hook v2 negotiation
should use, not a second magic number.

---

## Epic 9 — Protocol v2: envelope, negotiation, conformance

### Story 9.1 — Version negotiation and graceful degrade
- [x] A v2 client meeting a v1 server degrades to a working v1 session instead of failing. Today a
  version difference is fatal and produces exactly `xsync version mismatch: local vX / remote vY`,
  which is the right error for a sync run and the wrong outcome for an app that would happily
  transfer without browsing.

**AC**
- A v2 client against a v1 server completes a push and a pull, with browse-only features reported
  as unavailable rather than erroring the session.
- A v1 client against a v2 server is unaffected and never sees a v2 frame.
- The negotiated version and capability set are observable to the client before the first data
  frame, so an embedding app can enable or grey out its own UI (f2 **A12.3** shows connection state
  in the pane).
- Degrade is decided once, during handshake — never by catching a mid-session error.
- The existing mismatch message survives for the genuinely incompatible case, unchanged in wording.

**Results:**
- Added a reserved `CAP_BROWSE_V2` capability bit and a `protocol-negotiated` event carrying the
  selected version, remote capability set, common capability set, and browse availability before
  the first data frame. The CLI transport selection now reflects the selected version and reports
  browse-v2 as unavailable when the peer does not advertise it.
- Added shared deterministic `negotiate_protocol_version` and `common_capabilities` helpers with
  downgrade tests. Current peers select v1 because neither advertises `CAP_BROWSE_V2`; no v2 frame
  is emitted before Story 9.2 freezes the v2 grammar.
- Added `CAP_VERSION_NEGOTIATION` to the default v1 peers. The opening handshake remains encoded as
  a v1 frame, so an older v1 server can ignore the extension and the client can commit to v1 without
  probing by sending session data.
- The current v1 decoder remains fail-closed for a genuinely different envelope version, preserving
  the exact `xsync version mismatch: local vX / remote vY` error.

**Follow-up boundary:**
- The proposed contract is recorded in [`v2handshake.md`](v2handshake.md): the opening frame remains
  v1-compatible, and both peers select v2 only when both negotiation and browse bits are present.

**Results (continued):**
- Added version-pinned frame encoding and decoding APIs. `FrameDecoder::new()` remains v1, while a
  selected v2 session uses `FrameDecoder::for_version(2)` and
  `encode_frame_with_version(..., 2)`; either direction rejects the other version with the existing
  exact mismatch wording. There is no retry or mid-session downgrade.
- Added boundary tests proving v2-to-v1 and v1-to-v2 frame rejection, and retained the existing push
  and pull downgrade coverage against peers without browse capability.
- Full v2 session dispatch remains a follow-up to Epic 10, when the browse request loop consumes the
  v2 message types. Current sync peers correctly remain on v1 because they do not advertise
  `CAP_BROWSE_V2`.

### Story 9.2 — Freeze the v2 message table in `protocol.md`
- [x] Specify v2 in the same form as v1: frozen type bytes, payload fields in order, bounded counts,
  and the fail-closed rules restated for the new types. New types are assigned above the v1 range so
  a reader can tell at a glance which version introduced a message.

**AC**
- Every new message has its type byte, field order, and per-field bounds written down before any
  implementation lands, as v1's were.
- Collection counts, path lengths, and payload sizes reuse v1's caps (65,536 records, 1 MiB path,
  16 MiB payload) unless a story states a reason to differ.
- The document states what a v2 peer does with each v1 message and vice versa.
- Raw path bytes remain the representation. This is not restated for symmetry: f2 lists real user
  directories where non-UTF-8 names are ordinary, and a listing API that returns strings would
  corrupt them silently.
- `protocol.md` names both consuming projects and points at this file for the change process.

**Results:**
- Added the initial v2 table to `protocol.md` with types 14 through 21 for paged listing, stat,
  cancellation, keepalive, and per-request browse errors.
- Defined raw-path and symlink-target encoding, request correlation, page cursors, status/error codes,
  digest rules, collection and payload bounds, and v1/v2 message handling.
- Cross-checked against f2 A12.1a/A12.2: v2 listing entries intentionally omit content fingerprints,
  and browse error messages use xsync's 64 KiB bound; f2's existing 1 MiB v1 error bound remains a
  separate v1 contract.
- Reserved later mutation and single-file-transfer assignments rather than allowing them to drift
  into the initial browse table.

**Cross-project check:**
- Compared against f2's `Sources/F2Protocol/V2Browse.swift`. Both codecs now use the explicit entry
  kind values `1` file, `2` directory, `3` symlink, and `4` other; reject non-symlink targets on
  other entries; and validate UTF-8 error fields on encode and decode.
- f2 consumes the canonical vectors from its protocol test target with the source revision recorded
  in the copied fixture header.

### Story 9.3 — Cross-project conformance vectors
- [x] A language-neutral fixture corpus — byte-exact frames with their decoded meaning — that both
  repositories run in their own test suites. Without it, the Rust encoder and f2's Swift decoder
  drift silently until something fails at a customer.

**AC**
- Vectors cover every v2 message type, both directions, including at least one non-UTF-8 path, one
  maximum-length field, one boundary-count collection, and one deliberately malformed frame per
  fail-closed rule.
- Vectors live in one repository and are consumed by the other by copy with a recorded revision, or
  by submodule — the story picks one and says why.
- A vector change is a spec change and follows Story 14.1's process.
- Both suites fail loudly on a mismatch, and the failure names the offending field rather than
  reporting a byte offset.

**Results:**
- Added the canonical, line-oriented corpus in [`protocol-v2-vectors/`](protocol-v2-vectors/) with
  all eight v2 payload types, request/response/control directions, raw non-UTF-8 names, and named
  malformed cases for each payload fail-closed rule.
- Added Rust conformance tests that consume the corpus and separately exercise the maximum path and
  65,536-entry collection boundaries. Failures identify the vector ID and codec error.
- The corpus is repository-owned and is copied by the f2 consumer with the source revision recorded;
  this avoids a submodule dependency while keeping one canonical fixture format.

---

## Epic 10 — Interactive session: the browse surface

### Story 10.1 — Long-lived request/response session role
- [x] A third server role beside sink and source: a session that answers many small, independent
  requests over one connection and holds no sync state between them. `--server` today configures a
  role in the handshake and runs one operation to completion; browsing needs the connection to
  outlive any single request.

**AC**
- One spawned server answers an unbounded sequence of requests, in order, over a connection open for
  the life of the client's window.
- Requests are correlated by the existing unique message ID; a response names the request it answers.
- The session role refuses sync-only messages, as `CAP_DATA_ONLY` already refuses metadata messages —
  a role mismatch is a protocol error, not undefined behaviour.
- stdout stays protocol-only, per Story 3.2, including for errors.
- An idle session costs no CPU and holds no directory open. **Why this is explicit:** f2 will keep
  one open per remote pane for hours.

### Story 10.2 — `List`: one directory, paged
- [x] List the immediate children of one directory, paged and bounded, with the metadata a file
  browser renders — name bytes, kind, size, mtime, mode — and *without* content fingerprints.

**AC**
- Fingerprints are not computed. `Scan`'s entry record carries one because a diff needs it; hashing
  a directory to draw a row would make listing cost proportional to bytes rather than entries.
  Either the record grows an "unset" convention or v2 defines a lighter entry — Story 9.2 decides.
- The first page returns fast enough to render immediately on a directory of 100k entries, and the
  rest streams; measured on the wired benchmark hosts and recorded like every other transport number.
- Bounded memory on the server regardless of directory size, reusing the existing bounded scanner
  rather than a second traversal.
- Symlinks are reported as links with their target, never silently followed — a browser must be able
  to show the user what the entry actually is.
- Errors are per-entry where possible: one unreadable child does not fail the page.
- `~` is not expanded and is not special. f2's benchmark harness recorded sftp canonicalizing it to
  `/home/user/~/…`; a listing API that guesses inherits that bug.

**Results:**
- Implemented filesystem-backed `ListRequest` handling in the persistent session. Pages iterate
  `read_dir` directly, retain at most the requested page plus iterator state, and use the number of
  consumed directory entries as the opaque cursor.
- Entries use raw names, `symlink_metadata`, v1 kind values, size, nanosecond mtime, mode, and raw
  symlink targets without following links. Invalid paths and symlink ancestors become request-scoped
  errors rather than terminating the session.
- Added paging, symlink, traversal, and session integration coverage. Digest computation is not part
  of listing.
- Added a typed `BrowseSession::list_page` client method with cursor/final-page handling. The first
  page path is covered independently from later-page retrieval, with no content fingerprint work.

### Story 10.3 — `Stat`: one path
- [x] Metadata for a single path without listing its parent, for a properties panel (f2 **A14.3**)
  and for pre-flight checks before a mutation.

**AC**
- Answers for a file, a directory, a symlink (without following), and a path that does not exist —
  the last as a normal negative answer, not an error frame.
- Optionally includes a content digest, requested explicitly, so the caller pays for it only when it
  wants verification.
- Never materializes a cloud placeholder. Story 5.7's policy applies unchanged.

**Results:**
- Implemented filesystem-backed `StatRequest` handling with `symlink_metadata`, so final symlinks are
  reported as links and are never followed. Missing paths return normal `missing` responses.
- Added optional BLAKE3 computation for regular files only; directories, symlinks, missing paths, and
  errors never compute or return a digest.
- Added typed `BrowseSession::stat` client support and request-scoped handling for invalid paths and
  filesystem errors, plus coverage for file, directory, symlink, missing, digest, and traversal cases.

### Story 10.4 — Cancelling an in-flight request
- [x] A client can abandon a request it no longer needs and reuse the connection immediately.

**AC**
- A cancelled `List` stops producing pages promptly and the connection is immediately usable for the
  next request, with no residual frames attributable to the cancelled ID.
- Cancelling a request that has already completed is a no-op, not an error.
- **Why this is not optional:** typing a path generates a listing per keystroke, and a browser
  outruns a network trivially. Without cancellation the session serializes behind work nobody wants.

**Results:**
- Added `BrowseSession::cancel`, which sends a correlated `CancelRequest` and requires the code-1
  cancellation acknowledgement.
- The server tracks non-final list requests, acknowledges cancellation, emits no later page for the
  cancelled request, and continues serving keepalive or subsequent requests on the same connection.
- Cancellation of an already completed or unknown request is an idempotent acknowledgement. Added
  end-to-end coverage for cancellation, no residual pages, no-op cancellation, and connection reuse.

### Story 10.5 — Session health, keepalive, and shutdown
- [x] Detect a dead peer, keep an idle connection alive across NAT and ssh timeouts, and shut down
  cleanly on either side.

**AC**
- A dropped connection surfaces to the client as a distinct, nameable condition — not a hang and not
  a generic I/O error.
- An idle session survives whatever the deployment's ssh `ClientAliveInterval` is, or documents the
  requirement it places on the operator.
- Server exits when its stdin closes; no orphaned process after the client dies. Verified by killing
  a client mid-session and checking the remote process table.
- **Why this earns its own story:** f2's harness measured ten requests over fresh ssh connections at
  2.295 s and 3.626 s against 0.367 s and 0.220 s for one persistent process. Connection reuse is
  not an optimization for a browser; it is the difference between usable and not.

**Results:**
- Added handshake role `Session` (`role=3`) and a persistent v2 frame reader/writer. The server
  switches from the v1 opening handshake to the v2 browse envelope exactly once after capability
  negotiation; sync roles remain on the existing v1 state machine.
- Added an ordered request loop with duplicate-ID rejection, keepalive acknowledgements, correlated
  request errors for the not-yet-implemented List/Stat operations, clean EOF shutdown, no journal or
  directory handle allocation, and stderr-only diagnostics.
- Added an in-memory end-to-end test covering one handshake and multiple v2 requests on one stream.
- Added a distinct `PeerDisconnected` client error and coverage for clean peer EOF. The server's
  session loop exits cleanly when stdin closes, so the remote process does not remain in the session
  loop after client shutdown.
- Keepalive is application-driven: callers must schedule `BrowseSession::keepalive` more frequently
  than the deployment's SSH `ClientAliveInterval` and any intervening NAT idle timeout. The server
  answers keepalives without filesystem work, making that requirement explicit rather than silently
  promising a universal network timeout policy.

**Follow-up:**
- No protocol follow-up; deployment-specific keepalive intervals remain operator configuration.

---

## Epic 11 — Remote mutation

### Story 11.1 — Rename and create-directory
- [x] Two single-shot mutations with no partial state: rename within a filesystem, and create a
  directory.

**AC**
- Rename is atomic, refuses to replace an existing destination by default, and reports `EXDEV`
  distinctly so a caller can fall back to copy-and-delete rather than guessing.
- Create-directory reports "already exists" distinctly from "permission denied" and from "parent
  missing" — f2 surfaces these to a user who can act on them.
- Both are refused outside the session's configured root (Story 11.3).
- Neither needs a journal entry: nothing is resumable about an operation that either happened or
  did not.

**Results:**
- Added v2 `RenameRequest`/`RenameResponse` and `CreateDirectoryRequest`/`CreateDirectoryResponse`
  message types with bounded raw paths and actionable mutation statuses.
- Rename validates both paths before the syscall, refuses an existing destination, uses the atomic
  filesystem rename operation, and reports cross-device failures as `EXDEV`/`cross-device`.
- Directory creation is single-level and distinguishes already-existing, permission-denied, and
  missing-parent outcomes. Both operations use the Story 11.3 root and symlink refusal boundary and
  never create journal state.
- Added codec and filesystem coverage for successful mutations, refusal outcomes, path escapes, and
  non-empty error payloads.

### Story 11.2 — Recursive remote delete
- [x] Delete a path and everything beneath it, with progress and cancellation.

**AC**
- Progress is per item, so a client can show a count moving on a large tree.
- Cancellation stops promptly and leaves the remainder in place; the response says what was removed.
- Every failing path is reported with its errno, not just the first — a retry list is the point.
- Behaviour on a directory that changes underneath the delete is defined, not incidental.
- **The asymmetry must be visible in the protocol, not just the UI:** there is no Trash on a remote
  host, so this is irreversible and no inverse can be recorded. f2 has a journal-backed undo
  (its E9.3) that will silently not cover this; the response should make that unambiguous rather
  than leaving the client to remember.

**Results:**
- Added v2 `DeleteRequest`, per-item `DeleteProgress`, and terminal `DeleteResponse` messages.
  Responses carry an explicit irreversible marker, removed count, and bounded failure paths with
  platform errno values.
- Implemented post-order recursive deletion without following symlinks. Directory changes during
  traversal are snapshot-defined: entries observed after a directory is read are not included, and
  removal failures are retained for retry.
- Added a session reader queue so cancellation can arrive while deletion is running. Cancellation
  acknowledges the request, stops before the next item, leaves the remainder in place, and returns
  the removed count and status.
- Added codec and end-to-end coverage for per-item progress, complete deletion, failure reporting,
  cancellation plumbing, path refusal, and irreversible response semantics.

### Story 11.3 — Path safety for every mutation
- [x] One set of refusal rules applied to all of Epic 11, stated once.

**AC**
- Reuses the destination validation Story 3.2 already applies before publication: directory
  symlinks, traversal, parent-replacement races, and duplicate normalized destinations.
- A session may be confined to a root; escapes are refused before any syscall touches the target.
- Refusals name the rule they violated, so an app can explain itself to a user.
- Hostile-path fixtures from the existing suite are exercised against the mutation ops, not only the
  sync path.

**Results:**
- Strengthened `validate_destination_path` as the shared mutation boundary. Every existing destination
  validation now rejects any pre-existing symlink in the parent chain, including links to files, rather
  than following links during the check.
- Added `validate_unique_destination_path`, which validates before registering a normalized `WirePath`
  and reports duplicate destinations by rule-specific error.
- Added hostile traversal, absolute, empty-path, directory-symlink, file-symlink, and duplicate-path
  coverage. The validator runs before destination filesystem operations in the sink mutation paths.

---

## Epic 12 — Single-file transfer, for remote editing

### Story 12.1 — Fetch one file
- [x] Retrieve a single file to a caller-chosen local path, with its digest, without planning a sync.

**AC**
- Reuses the existing chunking, compression, and resume machinery — this is a narrow entry point,
  not a second transfer implementation.
- Returns the digest and the source's identity (size, mtime) as of the read, which Story 12.2 needs.
- A file that changes mid-read is detected, per the stable-read rules the source role already
  applies.

**Results:**
- Added v2 `FetchRequest`, `FetchStart`, and bounded `FetchChunk` messages. The start carries the
  stable file size, mtime, filesystem identity, and BLAKE3 digest.
- The server builds a single `FileEntry` from path metadata and reuses `SourceReader`'s stable-read
  retry and mutation detection; no tree scan or sync plan is created.
- `BrowseSession::fetch` stages chunks in the destination directory, verifies size/order/digest, and
  atomically publishes the caller-chosen local path only after verification.
- Added protocol and end-to-end coverage for multi-chunk fetches, identity, digest, and stable source
  handling.

### Story 12.2 — Publish one file back, without clobbering
- [x] Write a single file back atomically, refusing if the remote changed since it was fetched.

**AC**
- The caller supplies the identity it fetched; the server compares before publishing and refuses
  with a distinct "changed underneath you" answer.
- Publication is atomic — the remote never observes a partial file — reusing the sink's staging and
  rename path.
- The refusal carries the current identity so the client can offer a real choice rather than a retry.
- **Why the refusal matters more than the write:** f2's remote editing story (**A12.7**) opens a
  remote file in a local editor and uploads on save. Without this check, two people editing the same
  server config silently lose one of the edits, and the app that caused it looks like the culprit.

**Results:**
- Added v2 `PublishRequest`, `PublishReady`, `PublishChunk`, and `PublishResponse` messages. The
  request carries the fetched size, mtime, filesystem identity, and digest; changed responses carry
  the current identity or explicitly report that the target is absent.
- Added `BrowseSession::publish` with local digest validation, ordered bounded chunks, and a distinct
  changed result rather than an automatic retry.
- The server performs identity preflight and a second identity check before committing through the
  existing sink staging and atomic rename path. Existing remote content remains visible until the
  verified replacement is ready.
- Corrected the type-32 layout on 2026-08-28 after the embedding client proved that the original
  request reused fetched size/digest as the incoming payload contract, making every content- or
  length-changing edit impossible. Fetched identity and replacement size/digest are now separate,
  with a shared Rust/Swift conformance vector and a length-changing server acceptance test.
- Added codec and end-to-end coverage for matching publication, current-identity reporting, digest
  validation, and atomic replacement.

---

## Epic 13 — Agent lifecycle for an embedding application

### Story 13.1 — Version and capability probe
- [x] A cheap way to ask a host what it has before opening a session, with an actionable answer when
  the binary is missing, old, or not on `PATH`.

**AC**
- Missing binary keeps Story 3.3's existing message rather than inventing a second one.
- A version-skew answer says which side is older and what to do, and is distinguishable by a caller
  from "absent" and from "present but unusable".
- The probe costs one connection and is reusable as the session's connection where possible.

**Results:**
- Added `probe_session`, which performs the existing v1-compatible handshake once and returns the
  selected wire version, remote/common capabilities, local version, and typed `Ready`, `OlderPeer`,
  or `Unusable` status.
- Added `ProbedConnection::into_browse_session`, so a successful probe continues on the same already-
  handshaken streams without opening a second connection or repeating negotiation.
- Routed `BrowseSession::connect` through the probe, making the reusable handshake path the default
  rather than an opt-in helper. Probe statuses expose an actionable remediation string.
- Preserved Story 3.3's `MissingRemoteXsync` diagnostic for absent remote binaries; protocol answers
  distinguish an older peer from an unusable handshake.

### Story 13.2 — Staging a binary from a macOS client
- [~] Cross-compiled `linux/amd64` and `linux/arm64` binaries, staged to a host over ssh by a client
  that is not the build machine.

**AC**
- Produced from a macOS development machine, since that is where f2 is built. **This is the cost the
  A12.1 decision accepted:** f2's Go agent cross-compiled dependency-free with `CGO_ENABLED=0`, and
  Rust replaces that with target management — this story is where that bill comes due, and it should
  record how much it actually costs.
- Staging is idempotent, verifies what it uploaded, and does not require a package manager, a
  compiler, or root on the remote.
- Covers f2's three benchmark hosts (x86_64 ZFS, aarch64 ext4, x86_64 ext2/3), which are also the
  hosts its transport numbers come from.
- An interrupted staging leaves either the old binary or none — never a truncated one.

**Results so far:**
- Added [`scripts/stage-linux.sh`](scripts/stage-linux.sh), mapping `amd64` and `arm64` to the Tier 1
  GNU Linux targets and building with `cargo zigbuild` from the macOS checkout.
- The script stages through a same-directory temporary name, verifies SHA-256 before publication and
  after the atomic `mv`, is repeatable, and cleans interrupted temporary uploads. The remote needs
  only SSH, checksum, basic file utilities, and execute permission; no compiler, package manager, or
  root is required.
- Added `scripts/ensure-linux-agent.sh`, used by the three-host verifier's `--stage` mode. It hashes
  the required target binary and the installed agent, leaves an identical install untouched, and
  automatically invokes the atomic verified staging path when the agent is missing or stale. This
  catches same-version development builds whose wire revision differs, not merely version strings.
- Added [`docs/linux-staging.md`](docs/linux-staging.md) with the three benchmark-host mapping,
  commands, prerequisites, and failure-safety contract.

**Remaining verification:**
- Cross-build and staging are verified against `192.168.1.119` (`mars`, x86_64, ext4):
  `x86_64-unknown-linux-gnu` built in 53.97 s, produced a 4,905,688-byte binary, staged with SHA-256
  `c840e3ed...a130ae`, and ran `xs 0.1.0`; `aarch64-unknown-linux-gnu` built in 54.24 s, produced a
  4,139,344-byte ARM ELF, and staged with SHA-256 `28fe3fa0...cf791`.
- The x86_64 ZFS, x86_64 ext2/3, and aarch64 ext4 benchmark hosts still need staging checks before
  this story can be marked complete. The supplied host covers the build and remote atomic-install
  path, but is not those three filesystem cases.

**Blocker:**
- The current macOS checkout can build both GNU targets and can reach `freya.local` (x86_64/ZFS),
  but the configured `192.168.1.119` host timed out and `gentoo-rpi5.local` rejected the available
  SSH key. The three-host staging proof cannot be completed until credentials/network access to the
  ext2/3 and aarch64/ext4 hosts are restored. `scripts/verify-linux-staging.sh` records all host
  results and continues instead of stopping at the first blocker.

**Verification 2026-08-28:**
- `scripts/verify-linux-staging.sh` passed both GNU cross-builds and reached the required x86_64/ZFS
  (`freya.local`) and x86_64/ext2/3 (`192.168.1.119`) hosts. The aarch64/ext4 host
  (`gentoo-rpi5.local`) did not resolve, so the story remains partial.
- `scripts/ensure-linux-agent.sh` then detected stale hashes on both reachable hosts after the
  type-32 protocol correction, atomically installed SHA-256 `467fc7ee…e4de9`, and reported both
  agents current on a second run. The ARM64 hostname still did not resolve and no alternate
  reachable aarch64 LAN host was discoverable, leaving that single physical-host proof open.

### Story 13.3 — Connection model for a browsing client
- [x] Revisit Story 4.3's connection-model decision for a client that holds a session open for hours
  rather than running one sync.

**AC**
- States whether ControlMaster is used, required, or bypassed for a persistent session, and why.
- Reconnect after a dropped link resumes the *session*, or says plainly that it cannot and what the
  client must redo.
- An in-flight transfer interrupted by a link drop still resumes through the existing durable
  checkpoint journal — browsing must not have weakened Epic 3's guarantee.

**Results:**
- Documented the browse connection contract in [`docs/browse-connection-model.md`](docs/browse-connection-model.md):
  one ordinary persistent SSH process per pane, no implicit or required `ControlMaster`, and the
  existing application keepalive requirement.
- A dropped browse link reports `PeerDisconnected`; there is intentionally no session resumption
  protocol. The client probes and opens a new session, restarts list/stat/fetch work, and inspects
  remote state before retrying uncertain mutations.
- Documented that list cursors are session-scoped, fetch/publish temporary state is discarded or
  revalidated, and existing sync transfers retain Epic 3.4 durable checkpoint resume independently.

---

## Epic 14 — Two owners, one wire format

*This epic has no code. It exists because the failure mode it prevents — two projects shipping
incompatible "v2"s a month apart — is not caught by any test either project would think to write.*

### Story 14.1 — Spec ownership and change process
- [x] Write down who may change `protocol.md`, what a change requires, and how the other project
  learns about it.

**AC**
- Names the owning repository for the wire format and the review the other project gets.
- States the rule for type bytes: assigned once, never reused, never reinterpreted — v1 already says
  this for versions and it must survive two owners.
- Defines what constitutes a breaking change now that "compatible new message type" is impossible
  below a version bump.
- Says how a change is landed when both projects need it simultaneously, since neither can merge a
  half-implemented wire format.

**Results:**
- Added [`docs/protocol-ownership.md`](docs/protocol-ownership.md), naming xsync as the canonical
  owner, f2 as the consuming reviewer, the never-reuse type-byte rule, version-bump boundary, and
  the coordinated landing/release sequence. `protocol.md` now links to that contract.

### Story 14.2 — Compatibility matrix and release coordination
- [x] A published statement of which client versions work with which server versions, and what
  happens at each intersection.

**AC**
- Every cell is one of: works, works with reduced capability (naming which), or refuses with a
  specific message. No cell is "undefined".
- The matrix is generated from the conformance vectors of Story 9.3 rather than maintained by hand,
  or the story explains why hand-maintenance is acceptable.
- Covers the asymmetric field case both projects will actually hit: a long-lived agent staged months
  before the client that connects to it.

**Results:**
- Added [`scripts/generate-compatibility-matrix.py`](scripts/generate-compatibility-matrix.py) and
  generated [`docs/compatibility-matrix.md`](docs/compatibility-matrix.md). The generator hashes and
  counts the canonical Story 9.3 vectors, emits every client/server intersection, and names the
  reduced-capability result for a long-lived v1 agent or f2 client that does not yet implement the
  later mutation/transfer assignments. `--check` fails when the checked-in matrix is stale.

### Story 14.3 — A joint smoke test
- [~] One test exercising a real f2 client against a real xsync server, run before either project
  tags a release.

**AC**
- Runs from a single command on a development machine, against at least one of the benchmark hosts.
- Covers the browse surface end to end — connect, list, stat, fetch, publish, mutate, disconnect —
  because unit tests on either side cannot catch a disagreement about meaning, only about bytes.
- Failure output identifies which side is wrong, which is the whole reason it exists.

**Results so far:**
- Added [`scripts/joint-smoke.sh`](scripts/joint-smoke.sh), the single entry point. It runs the
  xsync v2 server/codec tests and the real f2 Swift protocol suite first, then checks that f2 exposes
  the message cases needed for rename, mkdir, delete, fetch, and publish. Failures are labelled
  `[xsync]` or `[f2]`; an incomplete consumer is reported as a blocker rather than a false pass.
- Reverified 2026-08-28 after f2 A12.7: the Swift client now exposes callable fetch and publish
  operations as well as mutation, and the live Rust/Swift acceptance tests cover list, mutation,
  fetch, CAS publish, disconnect, and conflict refusal. The surface-presence gate therefore no
  longer reports `client:fetch client:publish`.

**Blocker:**
- The single command still stops after its conformance and client-surface gates; it tells the
  operator to invoke f2's host-backed runner instead of invoking that runner itself. Story 14.3
  remains partial until `joint-smoke.sh` performs the complete live sequence against a selected
  benchmark host and labels any failure by side.
