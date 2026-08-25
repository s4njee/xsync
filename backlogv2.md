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
- [ ] A v2 client meeting a v1 server degrades to a working v1 session instead of failing. Today a
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

### Story 9.2 — Freeze the v2 message table in `protocol.md`
- [ ] Specify v2 in the same form as v1: frozen type bytes, payload fields in order, bounded counts,
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

### Story 9.3 — Cross-project conformance vectors
- [ ] A language-neutral fixture corpus — byte-exact frames with their decoded meaning — that both
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

---

## Epic 10 — Interactive session: the browse surface

### Story 10.1 — Long-lived request/response session role
- [ ] A third server role beside sink and source: a session that answers many small, independent
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
- [ ] List the immediate children of one directory, paged and bounded, with the metadata a file
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

### Story 10.3 — `Stat`: one path
- [ ] Metadata for a single path without listing its parent, for a properties panel (f2 **A14.3**)
  and for pre-flight checks before a mutation.

**AC**
- Answers for a file, a directory, a symlink (without following), and a path that does not exist —
  the last as a normal negative answer, not an error frame.
- Optionally includes a content digest, requested explicitly, so the caller pays for it only when it
  wants verification.
- Never materializes a cloud placeholder. Story 5.7's policy applies unchanged.

### Story 10.4 — Cancelling an in-flight request
- [ ] A client can abandon a request it no longer needs and reuse the connection immediately.

**AC**
- A cancelled `List` stops producing pages promptly and the connection is immediately usable for the
  next request, with no residual frames attributable to the cancelled ID.
- Cancelling a request that has already completed is a no-op, not an error.
- **Why this is not optional:** typing a path generates a listing per keystroke, and a browser
  outruns a network trivially. Without cancellation the session serializes behind work nobody wants.

### Story 10.5 — Session health, keepalive, and shutdown
- [ ] Detect a dead peer, keep an idle connection alive across NAT and ssh timeouts, and shut down
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

---

## Epic 11 — Remote mutation

### Story 11.1 — Rename and create-directory
- [ ] Two single-shot mutations with no partial state: rename within a filesystem, and create a
  directory.

**AC**
- Rename is atomic, refuses to replace an existing destination by default, and reports `EXDEV`
  distinctly so a caller can fall back to copy-and-delete rather than guessing.
- Create-directory reports "already exists" distinctly from "permission denied" and from "parent
  missing" — f2 surfaces these to a user who can act on them.
- Both are refused outside the session's configured root (Story 11.3).
- Neither needs a journal entry: nothing is resumable about an operation that either happened or
  did not.

### Story 11.2 — Recursive remote delete
- [ ] Delete a path and everything beneath it, with progress and cancellation.

**AC**
- Progress is per item, so a client can show a count moving on a large tree.
- Cancellation stops promptly and leaves the remainder in place; the response says what was removed.
- Every failing path is reported with its errno, not just the first — a retry list is the point.
- Behaviour on a directory that changes underneath the delete is defined, not incidental.
- **The asymmetry must be visible in the protocol, not just the UI:** there is no Trash on a remote
  host, so this is irreversible and no inverse can be recorded. f2 has a journal-backed undo
  (its E9.3) that will silently not cover this; the response should make that unambiguous rather
  than leaving the client to remember.

### Story 11.3 — Path safety for every mutation
- [ ] One set of refusal rules applied to all of Epic 11, stated once.

**AC**
- Reuses the destination validation Story 3.2 already applies before publication: directory
  symlinks, traversal, parent-replacement races, and duplicate normalized destinations.
- A session may be confined to a root; escapes are refused before any syscall touches the target.
- Refusals name the rule they violated, so an app can explain itself to a user.
- Hostile-path fixtures from the existing suite are exercised against the mutation ops, not only the
  sync path.

---

## Epic 12 — Single-file transfer, for remote editing

### Story 12.1 — Fetch one file
- [ ] Retrieve a single file to a caller-chosen local path, with its digest, without planning a sync.

**AC**
- Reuses the existing chunking, compression, and resume machinery — this is a narrow entry point,
  not a second transfer implementation.
- Returns the digest and the source's identity (size, mtime) as of the read, which Story 12.2 needs.
- A file that changes mid-read is detected, per the stable-read rules the source role already
  applies.

### Story 12.2 — Publish one file back, without clobbering
- [ ] Write a single file back atomically, refusing if the remote changed since it was fetched.

**AC**
- The caller supplies the identity it fetched; the server compares before publishing and refuses
  with a distinct "changed underneath you" answer.
- Publication is atomic — the remote never observes a partial file — reusing the sink's staging and
  rename path.
- The refusal carries the current identity so the client can offer a real choice rather than a retry.
- **Why the refusal matters more than the write:** f2's remote editing story (**A12.7**) opens a
  remote file in a local editor and uploads on save. Without this check, two people editing the same
  server config silently lose one of the edits, and the app that caused it looks like the culprit.

---

## Epic 13 — Agent lifecycle for an embedding application

### Story 13.1 — Version and capability probe
- [ ] A cheap way to ask a host what it has before opening a session, with an actionable answer when
  the binary is missing, old, or not on `PATH`.

**AC**
- Missing binary keeps Story 3.3's existing message rather than inventing a second one.
- A version-skew answer says which side is older and what to do, and is distinguishable by a caller
  from "absent" and from "present but unusable".
- The probe costs one connection and is reusable as the session's connection where possible.

### Story 13.2 — Staging a binary from a macOS client
- [ ] Cross-compiled `linux/amd64` and `linux/arm64` binaries, staged to a host over ssh by a client
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

### Story 13.3 — Connection model for a browsing client
- [ ] Revisit Story 4.3's connection-model decision for a client that holds a session open for hours
  rather than running one sync.

**AC**
- States whether ControlMaster is used, required, or bypassed for a persistent session, and why.
- Reconnect after a dropped link resumes the *session*, or says plainly that it cannot and what the
  client must redo.
- An in-flight transfer interrupted by a link drop still resumes through the existing durable
  checkpoint journal — browsing must not have weakened Epic 3's guarantee.

---

## Epic 14 — Two owners, one wire format

*This epic has no code. It exists because the failure mode it prevents — two projects shipping
incompatible "v2"s a month apart — is not caught by any test either project would think to write.*

### Story 14.1 — Spec ownership and change process
- [ ] Write down who may change `protocol.md`, what a change requires, and how the other project
  learns about it.

**AC**
- Names the owning repository for the wire format and the review the other project gets.
- States the rule for type bytes: assigned once, never reused, never reinterpreted — v1 already says
  this for versions and it must survive two owners.
- Defines what constitutes a breaking change now that "compatible new message type" is impossible
  below a version bump.
- Says how a change is landed when both projects need it simultaneously, since neither can merge a
  half-implemented wire format.

### Story 14.2 — Compatibility matrix and release coordination
- [ ] A published statement of which client versions work with which server versions, and what
  happens at each intersection.

**AC**
- Every cell is one of: works, works with reduced capability (naming which), or refuses with a
  specific message. No cell is "undefined".
- The matrix is generated from the conformance vectors of Story 9.3 rather than maintained by hand,
  or the story explains why hand-maintenance is acceptable.
- Covers the asymmetric field case both projects will actually hit: a long-lived agent staged months
  before the client that connects to it.

### Story 14.3 — A joint smoke test
- [ ] One test exercising a real f2 client against a real xsync server, run before either project
  tags a release.

**AC**
- Runs from a single command on a development machine, against at least one of the benchmark hosts.
- Covers the browse surface end to end — connect, list, stat, fetch, publish, mutate, disconnect —
  because unit tests on either side cannot catch a disagreement about meaning, only about bytes.
- Failure output identifies which side is wrong, which is the whole reason it exists.
