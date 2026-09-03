# xsync v3 — Network filesystem service

**Purpose.** Specify, as epics and user stories with acceptance criteria, everything
xsync needs so that a client can use an xsync server *the way it uses an NFS export or
an SMB share*: connect with an identity, see what is shared and whether it is writable,
browse large trees, read and write files at arbitrary offsets, keep a directory view
coherent while others change it, lock what it is editing, and survive a dropped link.

The first consumer is **Excalibur** (`~/projects/excalibur`, see `plan.md`), a
Rust/Tauri/SolidJS share browser whose SMB, NFS, SFTP and xsync backends sit behind one
trait. The second is **Kestrel** (`~/projects/sftp`), whose review of xsync
(`../sftp/backlog-xs.md`) identified the same gap this document closes:

> `xs` has no concept of "open this file at offset 40960 and stream bytes", which is
> what browsing, previews, edit-and-sync and resumable single-file downloads are built on.

This document is written against the xsync tree at commit `1bdcfd71`
(2026-09-02). It follows the change process in `docs/protocol-ownership.md`: nothing
here is a wire contract until it lands in `protocol.md` with vectors, codec tests, a
compatibility-matrix row and the f2/Kestrel consumer decision.

---

## Progress

| Phase | Story | Status |
|---|---|---|
| 1 | E11-S1 — v3 freeze (Phase 1 table) | **Done** 2026-09-03 |
| 1 | E3-S6 — Capability negotiation | **Done** 2026-09-03 |
| 1 | E3-S7 — Error model (Phase 1 scope) | **Done** 2026-09-03 |
| 1 | E3-S1 — Concurrent requests | **Done** 2026-09-03 |
| 1 | E1-S4 — Mount and advertised writability | Next |
| 1 | E4-S1–S3, E4-S8, E5-S1–S3, E9-S1/S4/S6, E10-S1/S4 | Not started |
| 2–6 | everything else | Not started |

All of the above is on xsync branch `v3`, **uncommitted**. §0 below still describes the
tree as it was *before* this work: it is the baseline the gap was measured against, and
is deliberately left unedited.

---

## 0. Where xsync stands today

Facts, with file references, so the gap is measured rather than assumed.

| Area | Today | Where |
|---|---|---|
| Transport | Spawned `ssh host xs --server <root>`; stdio pipes. No TCP listener, no TLS, no daemon. | `server.rs:3620 run_server_stdio`, `docs/browse-connection-model.md` |
| Authentication | None of its own — whatever OpenSSH accepted. Server runs as the SSH login user. | README §7 "SSH, and what xsync deliberately does not touch" |
| Authorization | One root path per process; path confinement below it. No exports, no `ro`/`rw`, no identity mapping. | `server.rs:1064 validate_destination_path`, `--server <root>` |
| Envelope | 32-byte `xsn1` header, 16 MiB payload cap, 8 MiB segment cap, optional zstd, unique message id, fail-closed. | `protocol.md` "Envelope" |
| Negotiation | v1 opening handshake; capability bits `CAP_DATA_ONLY`, `CAP_ZSTD`, `CAP_BROWSE_V2`, `CAP_VERSION_NEGOTIATION`, `CAP_BROWSE_META`; version selected once. | `v2handshake.md`, `server.rs:295 probe_session` |
| Browse (v2, types 14–41) | `ListRequest/ListPage` (paged, opaque cursor), `StatRequest/StatResponse` (+BLAKE3), `Cancel`, `Keepalive`, `BrowseError`, `Rename` (no overwrite), `CreateDirectory` (no parents), `Delete` (recursive, never follows symlinks), `Fetch` (whole file, 1 MiB chunks), `Publish` (whole file, compare-and-swap on size+mtime+identity), `SetPermissions`, `SetMtime`, `ReadLink`. | `protocol.md` "v2 message table", `protocol_v2.rs:174 V2Message` |
| Entry record | path, kind (file/dir/symlink/other), size, mtime ns, mode, symlink target. **No uid/gid, nlink, atime/ctime, allocated size, inode.** | `protocol.md` "v2 field encoding" |
| Concurrency | "The server processes requests in arrival order for v2." One request at a time per session; responses carry `related request ID` but are never reordered. | `protocol.md` |
| Session | No resumable identity; a drop is `PeerDisconnected` and *everything* is redone from scratch. | `docs/browse-connection-model.md` "Reconnect" |
| Random access | **None.** Fetch and Publish are whole-file. No open/close, no offset read, no offset write, no truncate, no fsync. | Kestrel `backlog-xs.md` XS-C1 |
| Capacity | **None.** No statfs / free-space message. | — |
| Locking | **None.** CAS publish is the only coherence primitive. | `protocol.md` PublishRequest |
| Change notification | **None.** | README §14 "v2 and v3 horizon" (persistent index is aspirational) |
| Metadata preserved | mtime, mode bits, empty dirs, symlinks. Hardlinks, uid/gid, ACLs, xattrs, resource forks, devices dropped (counted and reported). | README §10 |
| Sparse files | Read/written dense at logical size ("catastrophic", 28.6× measured). | README §10 |
| Client API | Synchronous `BrowseSession<R: Read, W: Write>` with typed helpers; `probe_session`; no async, no CLI exposure. | `server.rs:381`, README §9 "The library API" |
| Windows | Client only; cannot be the remote end without a POSIX shell. | README §10 |
| Governance | Type bytes frozen forever; new messages need a new version or a frozen capability-gated assignment; f2 keeps copied vectors. | `docs/protocol-ownership.md` |

**One-line summary:** xsync today is an excellent *tree synchroniser* with a small,
serial, whole-file *browse* surface bolted on. A network filesystem is the opposite
shape — many small concurrent random-access requests over an authenticated long-lived
session — and every row marked **None** above is a hard requirement for it.

---

## 1. Definition of "usable like NFS or Samba"

The table below is the exit criterion for this document. Every row must resolve to
a delivered story (or an explicit, documented "not supported, and here is how a
client finds out").

| Capability an NFS/SMB client takes for granted | NFS | SMB | xsync v3 story |
|---|---|---|---|
| Discover what a server shares | `showmount -e` / `EXPORTS` | `NetShareEnum` / tree connect | E1-S3 |
| Connect with an identity the server maps to a uid | `AUTH_SYS`, `krb5` | NTLM / Kerberos session setup | E2-S3, E2-S4 |
| Encrypted, integrity-protected wire | `krb5p`, RPC-with-TLS | SMB3 signing + `aes-128-gcm` | E2-S1 |
| Know if the share is writable before trying | export options `ro`/`rw` | tree-connect share flags / `MaximalAccess` | E1-S4, E5-S7 |
| Many requests in flight over one connection | RPC XIDs, `COMPOUND` | message ids + credits | E3-S1, E3-S3, E8-S2 |
| Survive a dropped connection | stateless (v3) / session + `RECLAIM_COMPLETE` (v4) | durable/persistent handles, session reconnect | E3-S2, E6-S5 |
| Open a file and read at an offset | `READ(fh, offset, count)` | `CREATE` + `READ` | E4-S1, E4-S2 |
| Write at an offset, then flush | `WRITE` + `COMMIT` | `WRITE` + `FLUSH` | E4-S3 |
| Truncate / preallocate / punch holes | `SETATTR size`, `ALLOCATE`/`DEALLOCATE` (v4.2) | `SET_INFO EndOfFile`, `FSCTL_SET_ZERO_DATA` | E4-S4 |
| Full attributes (owner, times, link count, inode) | `GETATTR` | `QUERY_INFO` | E5-S1 |
| Directory listing with attributes, large directories | `READDIRPLUS` | `QUERY_DIRECTORY` | E5-S2 |
| Free space / capacity | `FSSTAT` | `QUERY_FS_INFO` | E5-S3 |
| rename-with-overwrite, unlink, rmdir, symlink, link, chown, utimes | yes | yes (symlink via reparse) | E5-S4 |
| Extended attributes / named streams | `GETXATTR` (v4.2) | EAs / alternate data streams | E5-S5 |
| Ask "may I do X to this path" | `ACCESS` | `MaximalAccess`, `SD` query | E5-S7 |
| Byte-range locks | NLM / v4 `LOCK` | `LOCK` | E6-S1 |
| Cache safely with server-granted leases | delegations (v4) | oplocks / leases | E6-S2 |
| Learn when a directory changed | (polling / delegations) | `CHANGE_NOTIFY` | E7 |
| Compound several ops in one round trip | `COMPOUND` (v4) | compounded requests | E8-S2 |
| Saturate a LAN with streaming reads | large `rsize`, pipelining | multi-credit reads | E8-S3 |
| A client library other software can embed | libnfs | libsmbclient | E9 |
| Runs as a service with a config file | `nfsd` + `/etc/exports` | `smbd` + `smb.conf` | E1-S1, E1-S2, E12 |
| Mountable by the OS itself | kernel client | kernel client | E13 (stretch) |

---

## 2. Design principles

1. **Coexist, don't replace.** The sync protocol (v1/v2) and browse v2 keep working
   unchanged. v3 is selected by capability negotiation exactly as v2 is today; a v3
   client against a v2 server gets browse v2 and reports reduced capability. A v2
   client never sees a v3 frame.
2. **Fail closed, everywhere.** Every new message has bounds checked before
   allocation, unknown types abort the session, no fallback is ever inferred from a
   decode failure. This is xsync's existing discipline and it does not relax.
3. **The server is the source of truth for permission.** Writability, ownership and
   effective access are computed on the server and *advertised*; a client never has to
   attempt a write to discover it will fail. (Excalibur's `mount.writable` derives from
   this, never from the dialog.)
4. **Identity is explicit.** Every session has an authenticated principal and a
   documented mapping to filesystem identity. There is no ambient "whoever ran sshd".
5. **Concurrent by default.** A session handles many outstanding requests; responses
   are correlated by request id and may arrive out of order except where a handle's
   ordering guarantee says otherwise.
6. **Verifiable bytes.** Reads and writes may carry BLAKE3 digests per chunk; commit
   operations verify the whole file. Integrity is xsync's differentiator and it must
   survive the move to random access.
7. **Every claim is measured.** Performance stories carry a benchmark and a target;
   the existing harness (`xsync-bench`, `TESTING.md`) is extended rather than
   bypassed.
8. **Type bytes are assigned once.** v3 numbering starts at 42 and follows
   `docs/protocol-ownership.md`. The tables in §4 are *proposals* until frozen.

---

## 3. Epics and stories

Story ids are `X3-E<n>-S<m>`. Each story lists **AC** (acceptance criteria) that are
checkable, and **Depends** where sequencing matters. A story that has landed is marked
**Done** with its date and branch, its AC boxes ticked (or left unticked with the reason),
and a **Results** / **Next steps** pair recording what actually happened and what it
leaves for later — so a reader coming back cold does not have to reconstruct it from
git history. "Client" means any consumer of
`xsync-client` (E9); "server" means the daemon or `xs --server`.

### E1 — Daemon and exports

*Today: one `xs --server <root>` process per SSH login, no configuration, no notion of
a share.*

#### X3-E1-S1 — A long-running service
As an operator I can run xsync as a service so clients can connect without SSH.

**AC**
- `xs serve --config <file>` (or an `xsd` binary; decide once, document once) binds a
  TCP listener on a configurable address/port, accepts many concurrent sessions, and
  runs until signalled.
- `SIGHUP` (or a `reload` subcommand) reloads the exports file without dropping
  established sessions; a malformed reload is rejected and the previous config stays
  active, with the error logged.
- Graceful shutdown drains in-flight requests up to a timeout, notifies sessions
  (`Shutdown` message, E3-S7), then exits non-zero if any session was cut.
- Ships `systemd` unit and `launchd` plist examples under `packaging/`; the service
  runs as an unprivileged user by default (see E10-S6).
- Integration test: start the daemon in-process, connect two sessions, reload, confirm
  both still answer.

#### X3-E1-S2 — Exports configuration
As an operator I can declare what is shared and how, the way `/etc/exports` and
`smb.conf` do.

**AC**
- TOML file; each `[[export]]` has `name`, `path`, `access = "ro" | "rw"`,
  `allow` (list of principals/groups or `"*"`), `squash = "none" | "root" | "all"`,
  optional `map_to = { uid, gid }`, `guest = bool`, optional `comment`, optional
  `options` free-text string that is passed to clients verbatim (this is what
  Excalibur shows as "export mapped `ro,root_squash`").
- Export `path` must exist and be a directory at load; symlinked roots are resolved
  once at load and then confined.
- Duplicate names, overlapping paths with conflicting access, and unknown keys are
  load errors (fail closed, no "unknown key ignored").
- `xs serve --check-config` validates and prints the effective table without binding.
- Unit tests for every rejection above; a doc page `docs/exports.md`.

#### X3-E1-S3 — Export discovery
As a client I can list the exports a server offers before choosing one, like
`showmount -e` or SMB share enumeration.

**AC**
- `ListExports` request (pre-mount, after authentication) returns for each export
  the client is allowed to see: `name`, `access`, `comment`, `options`, `guest`.
- Exports the principal is not allowed to access are omitted (not shown as denied),
  unless the export sets `browseable_when_denied = true`.
- `xs exports host[:port]` prints the table; `--json` prints it as JSONL.

#### X3-E1-S4 — Advertised writability and mount facts
As a client I learn, at mount time and on every reconnect, whether I can write, and
why not if I can't.

**AC**
- `Mount` request names an export and returns a `MountInfo`: `export_name`, `access`
  (`ro`/`rw`), `effective_writable: bool` (access ∧ principal permitted ∧ filesystem
  not read-only), `reason` string when not writable (e.g. `export is ro`,
  `filesystem mounted read-only`, `principal squashed to nobody`), `options` string,
  `case_sensitive: bool`, `normalization: none|nfc|nfd`, `max_name_len`,
  `max_path_len`, `supports` bitmap (xattrs, symlinks, hardlinks, locks, leases,
  notify, sparse).
- The server re-evaluates `effective_writable` on reconnect (E3-S2) and whenever
  the export table is reloaded; a change is pushed as a `MountChanged` notification.
- A write-class request against an `ro` mount is refused with `EROFS` *before*
  touching the filesystem, and the refusal is not logged as an error.
- Test: flip an export from `rw` to `ro` via reload while a session is open; the
  session receives `MountChanged`, and its next write gets `EROFS`.

#### X3-E1-S5 — Session limits
As an operator I can bound what one session or one principal may hold.

**AC**
- Configurable per-session caps: open handles, active watches, held locks, in-flight
  requests, in-flight bytes; per-principal cap on concurrent sessions.
- Exceeding a cap returns a specific error (`ELIMIT` class, E3-S7) rather than
  stalling; the defaults are documented and sane (e.g. 1,024 handles, 256 watches,
  64 in-flight requests, 64 MiB in-flight bytes).

#### X3-E1-S6 — Windows as a server (later)
As an operator I can run the daemon on Windows.

**AC**
- `xs serve` runs as a Windows service; exports use Windows paths; non-UTF-8 name
  policy from `deferred.md` is decided (reject, not mangle).
- Explicitly out of scope for the first v3 release; recorded in the compatibility
  matrix as "not supported" with the exact refusal.

### E2 — Transport and authentication

*Today: SSH only, identity is the SSH login, no authentication or encryption of its
own.*

#### X3-E2-S1 — Native TCP transport with TLS 1.3
As a client I can connect directly to the daemon over an encrypted, authenticated
channel without SSH.

**AC**
- rustls TLS 1.3 only; server presents a certificate (self-signed by default,
  generated at first start, or operator-supplied). No plaintext TCP mode exists.
- Client verifies the server by **pinning**: on first connect the fingerprint is
  shown and stored (trust-on-first-use, exactly the SSH model Kestrel already
  implements); a changed fingerprint hard-fails and is never silently replaced. A CA
  mode (`--ca <file>`) is also supported for managed fleets.
- The negotiated cipher suite and server key type are exposed to the client
  (`SecurityInfo`) so a status bar can print e.g. `xsync3 · tls1.3-aes-128-gcm`.
- Default port registered in docs (proposal: `7394`); configurable.

#### X3-E2-S2 — SSH transport retained; bring-your-own stream
As a user with only SSH access I still get v3; as an embedding application I can run
v3 over a channel I already hold.

**AC**
- `ssh host xs --server <root>` continues to work and can negotiate v3 when both
  ends support it; the SSH login is the principal and identity mapping is the login
  user (no daemon needed).
- The client library accepts any `AsyncRead + AsyncWrite` pair (E9-S1) so Kestrel
  can drive v3 over its existing `russh` channel with no second connection.
- Compatibility matrix rows for `v3 client → v2 xs over ssh` (browse v2, reduced) and
  `v3 client → v3 xs over ssh` (full).

#### X3-E2-S3 — Authentication methods
As a user I can prove who I am to the daemon with a method my organisation already
uses.

**AC**
- Method negotiation after TLS: server advertises the methods it accepts for the
  connection; client picks one.
- **SSH public key**: challenge–response signed by the user's key (file or
  `ssh-agent`); server checks against per-principal `authorized_keys`-style entries in
  its config. This is the default and the one Excalibur's "no password for NFS"
  parity maps to.
- **Password**: verified against an argon2id credential file managed by
  `xs user add|passwd|rm`, *or* against PAM when built with the `pam` feature.
- **Token**: bearer token with expiry, for automation.
- **Guest**: allowed only on exports with `guest = true`; principal is `guest`.
- Failed attempts are rate-limited per source address; a lockout is logged.
- Passwords and tokens are zeroized after use and never logged.

#### X3-E2-S4 — Authorization and identity mapping
As an operator I control which principal reaches which export as which filesystem
identity — the `root_squash` / `all_squash` / `force user` family.

**AC**
- Each authenticated principal resolves to `(uid, gid, supplementary gids)` via the
  server's mapping table (`[[principal]]` entries) or, for SSH-transport sessions,
  the login user.
- Export `squash` is applied: `root` maps uid 0 → `map_to` (default `nobody`),
  `all` maps everyone → `map_to`, `none` leaves identities alone.
- Effective access checks use the mapped identity. When the daemon runs unprivileged
  (E10-S6) it *evaluates* mode bits/ACLs against the mapped identity itself and
  refuses what that identity could not do, even though the process could; when it
  runs privileged it uses per-session worker processes that `setresuid`/`setgroups`
  to the mapped identity so the kernel enforces it.
- Tests: an `all_squash` export shows files created by any principal as owned by
  `map_to`; a `root` principal cannot read a `0600` file owned by another user on a
  `root_squash` export.

#### X3-E2-S5 — Security labels for clients
As a client I can display the security posture of the connection.

**AC**
- `SecurityInfo` (returned after auth) carries: transport (`tls` | `ssh`), cipher,
  key algorithm and fingerprint, auth method, principal, mapped identity, and
  whether signing/encryption is on (always true for v3 — stated explicitly so a UI
  can print it).

### E3 — Session model

*Today: serial request processing, no session identity, everything restarts on a
drop.*

#### X3-E3-S1 — Concurrent requests and out-of-order responses — **Done**

*Landed 2026-09-03 on xsync branch `v3`.*

As a client I can keep many requests in flight and receive answers as they complete.

**AC**
- [x] Every request carries its own message id (already the case); every response and
  every notification carries `related request ID` (already the case for browse). v3
  *removes* the arrival-order processing rule: the server may execute independent
  requests concurrently and respond out of order.
- [x] Ordering guarantee: requests on the **same open handle** are applied in send
  order (a `Write` then `Read` on one handle observes the write). Requests on
  different handles or paths have no ordering guarantee; clients that need one wait
  for the response.
- [x] Server-side execution uses a bounded worker pool per session; the bound is the
  in-flight cap from E1-S5.
- [x] Test: issue 64 `Stat` requests without awaiting; all 64 responses arrive and ids
  match. *(The "≈ one RTT rather than 64 × RTT" half is asserted structurally instead
  of by clock: the test's handler parks every request until eight are inside it at
  once, so a serial dispatcher cannot complete it at all. A wall-clock assertion would
  be flaky on a loaded machine and would prove less.)*

**Results**

- `run_fs_session` is now a dispatcher rather than a loop. A reader thread, a bounded
  worker pool, and the session thread all meet on one `crossbeam_channel` carrying an
  `FsEvent` — either an inbound frame or a completed job. The session thread is the
  only writer, so responses cannot interleave on the wire even though they are produced
  concurrently.
- **Three things stay off the pool**: the `Features` exchange, `Keepalive` and
  `Cancel`. A keepalive queued behind a large read would report a busy session as a
  dead one, and a cancel that queues behind the work it is cancelling is useless.
  There is a test for each.
- **Per-handle ordering** is a queue per handle plus a busy set: at most one request
  per handle is dispatched at a time, and its successor is released when it answers.
  Requests without a handle (`Open`, path `Stat`, `StatFs`, `Mount`) never serialise.
  `fs_ordering_key` is the single place that decides which is which.
- **The in-flight cap** (`DEFAULT_FS_MAX_IN_FLIGHT`, 64) counts accepted-but-unanswered
  requests, queued ones included. Past it a request is answered `ELIMIT` and never
  executed. The server deliberately does *not* stall its reader to apply
  backpressure — that would take keepalive and cancel down with it. Credits are E3-S3.
- **Cancellation** is now precise: a queued request is removed and `ECANCELED` is its
  terminal response; a running one is flagged through `FsSessionState::is_cancelled`
  (which the E4/E5 handlers will poll) and its own response stays the only answer, so
  no request is ever answered twice.
- **A handler seam** — `trait FsHandler` plus `FsSessionState` (export root and the
  cancelled set, behind `Arc`) — exists so E1-S4/E4/E5 are written against a dispatcher
  that is already concurrent. The default `UnimplementedFsHandler` still answers
  `EOPNOTSUPP`. `Server::with_fs_limits` bounds a session for tests and operators.
- Six new tests, all deterministic — concurrency (a probe that only releases when eight
  requests are inside it), out-of-order replies (a slow request that finishes only
  after a later fast one), per-handle ordering (the first request fails the test if a
  successor overtakes it), the `ELIMIT` cap, keepalive overtaking pool work, and cancel
  removing queued work before it runs. Workspace green at 387 passed / 0 failed,
  `clippy -D warnings` and `fmt --check` clean.

**Next steps**

- **E1-S4 (`Mount`)** is the first real `FsHandler`, and the first verb to stop
  answering `EOPNOTSUPP`. It also decides how `MountInfo` reaches the handler — most
  likely a field on `FsSessionState` set once at mount.
- `FsSessionState` gains the handle table with **E4-S1**; that is the first shared
  mutable state the pool contends on, so it wants an `RwLock` and a test for
  concurrent `Open`/`Close`.
- **E3-S3 (credits)** replaces `ELIMIT`-on-overflow as the primary mechanism; the cap
  stays as the backstop for a client that ignores its window.
- The pool size (`DEFAULT_FS_WORKERS`, 8) is a guess until E8-S3 measures streaming
  reads. It is deliberately not derived from core count: these are I/O-bound.

#### X3-E3-S2 — Session identity, reconnect and resumption
As a client I can survive a dropped TCP/SSH connection without losing open files,
locks or watches.

**AC**
- `Mount` returns a `session_id` (128-bit random) and a `session_key` for
  re-authentication of the resume; the server keeps session state for a grace
  window (default 60 s, configurable) after a disconnect.
- `Resume(session_id, proof)` on a new connection reattaches: open handles, locks,
  leases and watches are restored; pending requests are *not* replayed (client
  responsibility, as today) but the client can query `HandleState` to learn the
  current size/offset of a handle it was writing.
- If the grace window expired, `Resume` fails with `ESTALE`-class error listing what
  was lost; the client re-mounts and re-opens.
- Session state is bounded (E1-S5) and evicted oldest-first under pressure.
- Test: kill the connection mid-write, reconnect within grace, finish the write,
  verify the file; repeat past grace and assert the documented failure.

#### X3-E3-S3 — Flow control
As a server I can prevent one client from flooding memory; as a client I can pipeline
without guessing.

**AC**
- Credit-based window negotiated at `Mount`: the server grants an initial credit
  count and byte window; each response returns credits; a client never has more
  outstanding requests/bytes than its credits (SMB2 credit model).
- Requests exceeding the window are a protocol error (fail closed), not a stall.
- The window adapts: a well-behaved client on a fast link is granted more (up to a
  configured max, e.g. 256 requests / 256 MiB).

#### X3-E3-S4 — Cancellation and timeouts
As a client I can abandon a request I no longer need.

**AC**
- `Cancel(related_id)` (exists) applies to every v3 request type, including in-flight
  reads and directory pages; the server stops sending further chunks and answers
  with the cancellation code.
- Per-request server-side deadline (optional field); expiry yields `ETIMEDOUT`.

#### X3-E3-S5 — Keepalive and idle policy
**AC**
- Keepalive interval negotiated at `Mount`; either side may send; three missed
  keepalives mark the peer dead and start the grace window.
- Idle sessions (no requests, keepalives only) are kept indefinitely unless the
  operator sets `idle_timeout`.

#### X3-E3-S6 — Capability negotiation for v3 — **Done**

*Landed 2026-09-03 on xsync branch `v3`.*

**AC**
- [x] New handshake bit `CAP_FS_V3` (bit 7). Version 3 is selected iff both
  sides advertise `CAP_VERSION_NEGOTIATION` and `CAP_FS_V3`; otherwise the existing
  v2/v1 selection applies unchanged.
- [x] After selecting v3, a `Features` exchange carries a v3 feature bitmap (locks,
  leases, notify, xattrs, sparse, compound, …) so optional epics can land
  independently; a client never sends a request whose feature bit the server did not
  advertise.
- [x] `protocol-negotiated` event gains `fs_v3_available` and `fs_v3_features`.
- [x] Matrix rows for every combination of v1/v2/v3 client and server. *(There are no
  wire **vectors** for selection: negotiation is a capability computation, not a
  payload. The equivalent coverage is the exhaustive unit matrix in
  `protocol.rs::negotiate_protocol_version` tests.)*

**Results**

- `negotiate_protocol_version` (`crates/xsync-core/src/protocol.rs`) selects 3 / 2 / 1,
  trying v3 first. **Only a `Role::Session` endpoint advertises `CAP_FS_V3`** — the sync
  client capability sets are untouched — so a push or pull negotiates exactly what it
  did before v3 existed. That is asserted directly rather than assumed
  (`negotiate_protocol_version(sync, v3) == 1`).
- Server: `Server::run` dispatches to a new `run_fs_session` when the selection is 3.
  The session opens with the `Features` exchange and then answers `Keepalive` and
  `Cancel`. `Server::with_fs_features` sets the server's optional bitmap, which is `0`
  today because every feature bit gates a later phase.
- Client: `probe_fs_session`, `ProbeStatus::ReadyV3`,
  `ProbedConnection::into_fs_session(requested_features)`, and an `FsSession` type with
  `negotiated_features` / `supports` / `require` / `send` / `receive` / `request` /
  `keepalive` / `cancel`.
- **`probe_session` is deliberately unchanged** and can never select v3. Without that
  split, upgrading a server would silently change the grammar under an existing browse
  consumer (Kestrel's `XsFs`); a regression test pins that a v2 probe against a v3-capable
  peer still selects v2 and still opens a browse session.
- `FsSession::require(features, name)` is the "never send an ungated request" guard: a
  fail-closed peer aborts the session on a message type it does not know, so this turns
  that into a local error naming the missing feature. The client also refuses a
  `FeaturesAck` granting anything it did not request.
- Filesystem verbs answer `Error { code: EOPNOTSUPP }` naming the message type — a
  per-request error, never a dead session.
- `LocalEvent::ProtocolNegotiated` and the `progress-json` event gained
  `fs_v3_available` and `fs_v3_features`.
- Tests: 5 new server tests (feature intersection and control verbs; the full
  client/server version matrix; the probe split and its regression guard; the client's
  negotiated-set handling; fail-closed frame order — `Features` not first, `Features`
  twice, duplicate message IDs, and a v2 frame after a v3 selection). Workspace green at
  381 passed / 0 failed, `clippy -D warnings` and `fmt --check` clean.

**Next steps**

- **E3-S1 (concurrent dispatch)** should land before any handler does: `run_fs_session`
  is serial today, and retrofitting concurrency under existing handlers is harder than
  building them on top of it.
- **E1-S4 (`Mount`)** is what makes the session useful to Excalibur; it is the first
  verb to replace its `EOPNOTSUPP`.
- Revisit reusing `EOPNOTSUPP` for "negotiated but not implemented". The frozen table
  has no better code and the message disambiguates, but a client could misread it as a
  filesystem limitation.
- `Server::fs_features` stays `0` until the first optional epic lands; the intersection
  logic is already exercised by tests that set it explicitly.

#### X3-E3-S7 — Error model — **Done (Phase 1 scope)**

*Landed 2026-09-03 on xsync branch `v3`.*

**AC**
- [x] One `Error` response shape for all v3 requests: `related request ID`, `code u16`
  from a frozen table mapped 1:1 to POSIX errno names where one exists (`ENOENT`,
  `EACCES`, `EROFS`, `EEXIST`, `ENOTEMPTY`, `EISDIR`, `ENOTDIR`, `EXDEV`, `ESTALE`,
  `ENOSPC`, `EDQUOT`, `ENAMETOOLONG`, `ELOOP`, `EBUSY`, `EWOULDBLOCK`,
  `ETIMEDOUT`, `ECANCELED`), plus xsync-specific `ELIMIT`, `ECHANGED` (CAS
  mismatch), `ELEASEBROKEN`; `platform_errno i32` when available; bounded UTF-8
  message.
- [ ] Server-initiated notifications (`MountChanged`, `LeaseBreak`, `WatchEvent`,
  `Shutdown`) use `related request ID = 0` and a distinct type range. **Deferred:** the
  type ranges are reserved (52–55, 100–109, 110–115, 120) and the `related = 0`
  convention is written into `protocol.md`, but no notification is defined, because each
  one belongs to the feature that raises it (E1-S4, E6-S2, E7, E1-S1).

**Results**

- 26 frozen codes as `protocol_v3::ErrorCode`, values `1..=26`, each with a `name()`
  returning its errno spelling for logs and generated docs. A test asserts every code's
  wire value equals its position in `ErrorCode::ALL`, that names are unique, and that
  `0` and `27` are rejected — so inserting a code in the middle later fails loudly
  instead of silently renumbering the table.
- `Error` (type 121) carries `related_id`, `code`, `platform_errno i32` (`0` when
  unavailable) and a bounded UTF-8 message. `Done` (type 122) was added alongside it for
  verbs with nothing to return (`Close`, `Flush`), so "succeeded" is never an empty
  `Error`.
- Three codes exist specifically for this protocol rather than POSIX: `EINTEGRITY` (a
  payload digest did not match), `ECHANGED` (a compare-and-swap precondition failed) and
  `ELEASEBROKEN`.

**Next steps**

- **E9-S4** maps these onto `std::io::ErrorKind` in the client, with the table generated
  from the enum so it cannot drift.
- Each notification type gets defined with the feature that raises it; none should be
  assigned speculatively.

### E4 — Random-access file I/O

*Today: whole-file `Fetch` and `Publish` only. This epic is the blocker Kestrel named
(XS-C1) and the one Excalibur's Quick Look, Edit-in-place, video preview and resumable
transfers all sit on.*

#### X3-E4-S1 — Open and close handles
As a client I can open a file and hold a handle across many operations.

**AC**
- `Open(path, flags, mode, attr_mask)` → `handle u64`, plus the file's attributes
  (E5-S1) and change cookie (E6-S4) so an open needs no extra stat.
- Flags: `READ`, `WRITE`, `CREATE`, `EXCL` (create-new), `TRUNC`, `APPEND`,
  `NOFOLLOW`, `DIRECTORY` (for directory handles used by E5-S2 and E7). Unknown flag
  bits are a protocol error.
- Mode applies only with `CREATE`; the server applies `mode & 0o7777` then the
  identity's umask policy (configurable per export).
- `Close(handle)` releases locks and leases held through it; closing an unknown
  handle is `EBADF`-class, not a session error.
- Handles are session-scoped and survive reconnect (E3-S2). All handles are closed
  when the session is finally torn down; the server never leaks descriptors
  (test with `lsof`-style count before/after 10,000 open/close cycles).
- A write-class `Open` against an `ro` mount is refused with `EROFS` before any
  filesystem call.

#### X3-E4-S2 — Positional read
As a client I can read any byte range of an open file, with pipelining.

**AC**
- `Read(handle, offset, length, want_digest)` → one `ReadData(related, offset, data,
  digest?)` response; `length ≤ max_read` negotiated at `Mount` (default 1 MiB, cap
  8 MiB to match `MAX_DATA_SEGMENT`); a short read is legal only at EOF and is marked
  `eof = true`.
- When `want_digest` is set the response carries BLAKE3 of `data`; the client
  library verifies before delivering.
- Reads on different handles, or non-overlapping reads on one handle, run
  concurrently on the server.
- Benchmark (E8-S3): 8 outstanding 1 MiB reads saturate 1 GbE from a warm page
  cache on the Mac→mars route the existing harness uses.

#### X3-E4-S3 — Positional write and flush
As a client I can write at an offset and know when the bytes are durable.

**AC**
- `Write(handle, offset, data, digest?)` → `WriteAck(related, bytes_written,
  new_size, change_cookie)`; `data ≤ max_write` (negotiated, same bounds as read).
- The server verifies the digest when present *before* the `pwrite`; a mismatch is
  `EINTEGRITY`-class and nothing is written.
- Semantics are **write-through to the OS page cache** (like NFS unstable writes):
  `Flush(handle)` → `fsync`; `WriteAck` carries a `stable: bool` that is `true` only
  if the export is configured `sync`.
- `APPEND` handles ignore `offset` and return the actual offset written.
- Test: interleave writes on two handles to one file; final content matches an
  in-process model; `Flush` then kill -9 the daemon; content persists.

#### X3-E4-S4 — Truncate, allocate, sparse
**AC**
- `SetSize(handle | path, size)` truncates or extends (extension is a hole).
- `Allocate(handle, offset, len, mode)` with `mode ∈ {ALLOCATE, PUNCH_HOLE,
  ZERO_RANGE}`; unsupported modes on the export's filesystem return `EOPNOTSUPP`
  and the `supports` bitmap (E1-S4) says so in advance.
- `SeekData/SeekHole(handle, offset)` for sparse-aware clients; `Stat` reports
  `allocated_size` (E5-S1). This is the primitive that finally lets sync stop
  writing 3.7 TB for a 130 GB image (README §10).

#### X3-E4-S5 — Access hints
**AC**
- `Advise(handle, offset, len, hint)` with `SEQUENTIAL`, `RANDOM`, `WILLNEED`,
  `DONTNEED` mapped to `posix_fadvise`/`F_RDADVISE` where available; no-op
  elsewhere; never an error.

#### X3-E4-S6 — Resumable, atomic uploads
As a client I can upload a large file in ranges, resume after a drop, and have the
destination appear atomically and verified — the property xsync's sink already gives
sync transfers.

**AC**
- `StageOpen(dest_path, size, digest?)` → `stage_id` and a resume token; the server
  stages in its existing deterministic temporary path under the destination
  directory (`sink.rs`), with the same crash-cleanup rules.
- `StageWrite(stage_id, offset, data, digest?)` ranges in any order; `StageStatus`
  returns the committed range set (reusing `RangeTracker` / `ResumePage` semantics).
- `StageCommit(stage_id, whole_file_digest, expect_cookie?)` verifies the complete
  BLAKE3, optionally checks the destination's change cookie (CAS, generalising
  `PublishRequest`), applies mode/mtime, then renames into place atomically.
  `StageAbort` discards.
- Stages survive reconnect (E3-S2) and daemon restart (token persisted next to the
  temp file) for a configurable retention (default 24 h), then are garbage-collected.
- Windows publication becomes crash-atomic (`BUGS.md` P1) as part of this story.

#### X3-E4-S7 — Compare-and-swap edits
As an editor I can save a file back only if nobody else changed it.

**AC**
- `StageCommit` with `expect_cookie` returns `ECHANGED` plus the current cookie and
  attributes when the destination moved; the client decides (overwrite, keep both,
  diff).
- `Write` accepts an optional `expect_cookie` for small in-place edits without
  staging.
- `Publish` (v2) is documented as the whole-file special case and remains available
  to v2 peers.

#### X3-E4-S8 — Handle lifecycle on failure
**AC**
- Handles on a path that is renamed remain valid (they reference the inode).
- Handles on a path that is unlinked remain readable until closed (POSIX), and
  `Stat(handle)` reports `nlink = 0`.
- After grace-window expiry every handle id from that session is invalid and any use
  returns `ESTALE`; ids are never reused within a daemon lifetime.

### E5 — Namespace and metadata

*Today: six-field entry record, no capacity, single-purpose mutations.*

#### X3-E5-S1 — Full attributes
**AC**
- `Attrs` record used by `Stat`, `Open`, `ReadDir`, `WriteAck`: `kind`, `mode`,
  `uid`, `gid`, `nlink`, `size`, `allocated_size`, `atime`, `mtime`, `ctime`,
  `btime?` (birth time when the FS has it), `dev`, `ino`, `rdev` (for device
  entries), `symlink_target?`, `change_cookie` (E6-S4), `flags` (immutable, append-
  only, hidden — for Windows/macOS hidden bit parity).
- Optional `owner_name`/`group_name` strings resolved server-side when the client
  sets the `names` bit of the request's `attr_mask`, so a UI can show names without
  an ID mapping of its own.
- `Stat(path, follow: bool)` = `stat`/`lstat`; `Stat(handle)` = `fstat`.
- Vectors cover every optional field present/absent.

#### X3-E5-S2 — Directory reading with attributes and stable cursors
**AC**
- `ReadDir(dir_handle, cursor, max_entries, attr_mask)` returns entries carrying the
  `Attrs` blocks the mask asked for (readdirplus), so a 10k-entry listing needs no
  per-entry stat.
- Cursors are server-side positions into a snapshot taken at first page; a page is
  O(page) not O(offset) — the O(n²) skip-from-start that Kestrel measured (167 ms at
  page 256 vs 46 ms at 16,384) is a bug to fix, with a regression test that pages of
  256 over 10k entries complete within 1.5× of a single page.
- Entries created/removed during the listing may or may not appear (snapshot), but
  no entry appears twice and none present at both start and end is missing.
- `.` and `..` are never returned; names are single components, raw bytes.
- Target: 10k-entry directory listed with attributes in < 100 ms on LAN (E8-S5).

#### X3-E5-S3 — Capacity and filesystem facts
**AC**
- `StatFs(export)` → `block_size`, `total_bytes`, `free_bytes`, `available_bytes`
  (to this identity, honouring quotas where the OS exposes them), `total_inodes`,
  `free_inodes`, `fs_type` string, `max_name_len`, `case_sensitive`, `normalization`,
  `read_only` (the *filesystem*, distinct from the export's `ro`).
- Excalibur's "2.4 TB free of 16 TB" reads `available_bytes` / `total_bytes`.

#### X3-E5-S4 — Complete mutation set
**AC**
- `Rename(src, dst, flags)` with `flags ∈ {NOREPLACE (today's behaviour), REPLACE,
  EXCHANGE}`; `REPLACE` is atomic on the same filesystem; `EXDEV` remains an error
  (no server-side copy fallback — the client does copy+delete explicitly).
- `Unlink(path)` removes exactly one non-directory; `Rmdir(path)` removes exactly one
  *empty* directory (`ENOTEMPTY` otherwise). Recursive `Delete` (v2) remains as the
  explicit bulk operation with progress.
- `Mkdir(path, mode)` (single level, as today); `MkdirAll` is a client-side loop.
- `Symlink(target, path)`, `Link(existing, new)`, `Chown(path|handle, uid?, gid?)`,
  `SetTimes(path|handle, atime?, mtime?, follow)` with `UTIME_NOW`/`UTIME_OMIT`
  semantics, `Chmod` (exists as `SetPermissions`).
- All mutations are refused with `EROFS` on `ro` mounts before any syscall and
  return the new `Attrs`/cookie on success.

#### X3-E5-S5 — Extended attributes
**AC**
- `ListXattr`, `GetXattr`, `SetXattr(flags CREATE|REPLACE)`, `RemoveXattr` on path
  or handle; names raw bytes, values ≤ 64 KiB (configurable), list ≤ 64 KiB.
- macOS resource forks and Finder info are just xattrs here; `com.apple.provenance`
  filtering is a client policy, not a protocol rule.
- Feature-gated (E3-S6); `EOPNOTSUPP` when the export's filesystem lacks them.

#### X3-E5-S6 — Path semantics
**AC**
- `MountInfo` (E1-S4) exposes case sensitivity and normalization probed with the
  existing `pathsem.rs` machinery, once per export at load, not per request.
- Paths remain raw bytes; on Windows servers the `deferred.md` question is resolved
  as **reject non-UTF-8 with `EILSEQ`**, never lossy.
- Intermediate symlink components are rejected as traversal (existing rule) unless
  the export sets `follow_symlinks_within_root = true`, in which case they are
  resolved with `openat2(RESOLVE_BENEATH)` on Linux and an equivalent walk on macOS.

#### X3-E5-S7 — Access query
**AC**
- `Access(path, mask)` with `R|W|X|DELETE|APPEND` returns the subset the mapped
  identity may perform, evaluated the same way the server would enforce it. This lets
  a UI grey out `Edit` for a single read-only file on a writable share.

### E6 — Locking and coherence

*Today: compare-and-swap publish only.*

#### X3-E6-S1 — Byte-range locks
**AC**
- `Lock(handle, offset, len, type READ|WRITE, wait: bool)` / `Unlock`; POSIX
  advisory semantics; owner is `(session, handle)`; conflicting `wait=false` returns
  `EWOULDBLOCK`; `wait=true` queues with fair ordering and is cancellable.
- Locks are released on `Close`, and on session teardown after the grace window.
- Locks are advisory to *other xsync sessions*; the server also takes matching
  `fcntl(F_OFD_SETLK)` locks so they are visible to local processes and to
  Samba/NFS servers exporting the same directory (documented as best-effort).
- `TestLock` reports the conflicting owner's principal for UI display.

#### X3-E6-S2 — Leases
**AC**
- `Open` may request a lease `READ`, `READ_HANDLE`, or `READ_WRITE`; the response
  grants the lease actually given (possibly `NONE`).
- Before another session (or a local change detected via E7) invalidates the lease,
  the server sends `LeaseBreak(handle, to_level)`; the client must `Flush`, drop
  cached data and `LeaseAck` within a timeout (default 5 s) or the server breaks it
  unilaterally and further cached writes return `ELEASEBROKEN`.
- Tests model SMB oplock break sequences: two sessions open the same file for
  write, the first's lease is broken before the second's `Open` returns.

#### X3-E6-S3 — Share modes (optional)
**AC**
- `Open` accepts `deny_read`/`deny_write`/`deny_delete`; conflicts return
  `EBUSY`-class with the holder's principal. Feature-gated; off by default.

#### X3-E6-S4 — Change cookie
**AC**
- Every `Attrs` carries an opaque 16-byte `change_cookie` derived from `(ino, size,
  mtime_ns, ctime_ns)` and, where the FS exposes one, the change attribute
  (`statx` `stx_attributes`/APFS `st_gen`-equivalent); equality means "unchanged".
- Used by `StageCommit`/`Write` CAS (E4-S7) and by clients as an ETag for their
  own caches.

#### X3-E6-S5 — Locks and leases across reconnect
**AC**
- Within the grace window (E3-S2), a resumed session finds its locks and leases
  intact; during the window they are held on its behalf and block other sessions.
- Documented trade-off: a crashed client blocks others for the grace window; the
  operator can shorten it per export.

### E7 — Change notification

*Today: none.*

#### X3-E7-S1 — Watch a directory
**AC**
- `Watch(dir_handle, recursive: bool, mask)` → `watch_id`; events `Created`,
  `Removed`, `RenamedFrom/To` (paired by cookie), `Modified`, `AttrChanged`, each
  with the relative name and, when cheap, the new `Attrs`.
- Events are delivered as notifications (`related = 0`, `watch_id` inside), coalesced
  within a short window (default 50 ms) so a 10k-file burst is not 10k frames.
- `Unwatch(watch_id)`; watches die with the session after grace.

#### X3-E7-S2 — Overflow is a rescan, not a lie
**AC**
- If the server's backend drops events (inotify `IN_Q_OVERFLOW`, FSEvents
  `MustScanSubDirs`/`UserDropped`, `notify` overflow) the client receives
  `WatchOverflow(watch_id)` and must relist; the server never pretends continuity.
  This is README §14's "correctness trap" made a contract.
- Test: generate 40k events faster than the consumer drains; assert overflow is
  delivered and no silent staleness.

#### X3-E7-S3 — Backend and limits
**AC**
- Implemented with the `notify` crate (already a Kestrel dependency); when the
  export root is itself a network filesystem where inotify is unreliable, a polling
  fallback (`PollWatcher`, configurable interval) is used and `MountInfo.supports`
  says `notify_polling`.
- Per-session watch cap (E1-S5); recursive watches on very large trees may be
  refused with `ELIMIT` and a hint to watch subdirectories.

### E8 — Caching and performance

#### X3-E8-S1 — Cache validity hints
**AC**
- `MountInfo` carries `attr_cache_ms` and `dir_cache_ms` recommendations (like NFS
  `acregmin`/`acdirmin`), and whether leases (E6-S2) are available so a client can
  cache longer under a lease.
- Client library implements an optional attribute cache honouring these.

#### X3-E8-S2 — Compound requests
**AC**
- `Compound([ops])` executes a short sequence server-side, stopping at the first
  error and returning per-op results; the canonical uses are `Open+Stat+Read`
  (first bytes of a preview in one RTT) and `Open(CREATE)+Write+Flush+Close` (small
  upload in one RTT). Max 8 ops; feature-gated.

#### X3-E8-S3 — Streaming read throughput
**AC**
- With `max_read = 1 MiB` and 8 in flight, a single-file read achieves ≥ 90% of
  the measured wire ceiling on the harness's 1 GbE route and ≥ 60% on the 10 GbE
  route (Phase 13 numbers in `backlogv4.md` are the baseline); results published in
  `benches/results/` alongside NFS and SMB reads of the same file from the same
  host pair.

#### X3-E8-S4 — Per-request compression
**AC**
- `Read`/`Write` payloads may be zstd-compressed under the existing frame flag,
  chosen by the existing adaptive sampler per request; incompressible media is never
  compressed twice.

#### X3-E8-S5 — Latency budgets
**AC**
- Published targets, measured by a new `xsync-bench fs` sub-harness:
  `Stat` ≤ 1 RTT + 1 ms; `ReadDir` 10k entries with attrs ≤ 100 ms LAN; first byte
  of a random 1 MiB read ≤ 2 RTT; `Open+Read` compound ≤ 1 RTT + disk.
- Each target has a regression gate in CI at the tolerance the harness already uses
  (median/MAD, paired arms).

#### X3-E8-S6 — Zero-copy research (spike)
**AC**
- A time-boxed spike measures `sendfile`/`splice`/`copy_file_range` for `Read`
  and server-side `Copy(src, dst)`; the outcome is a decision recorded in
  `backlog`, not a feature promise (the 4.7x null result in `backlogv4.md` is the
  precedent).

### E9 — Client library and tooling

*Today: synchronous `BrowseSession<R, W>`; no CLI browse commands.*

#### X3-E9-S1 — `xsync-client` async crate
**AC**
- New workspace crate, `tokio`-based, no Tauri/GUI dependency: `Client::connect_tls`,
  `Client::connect_ssh`, `Client::from_stream(AsyncRead + AsyncWrite)`; `Mount`
  handle exposing every E4–E7 operation as async methods returning typed errors.
- Object-safe trait `xsync_client::Fs` (list, stat, open/read/write/close, mkdir,
  rename, unlink, rmdir, statfs, watch, …) so Excalibur's `ShareBackend` and
  Kestrel's `RemoteFs` are thin adapters.
- Internally: one demultiplexing task per connection, request-id → oneshot map,
  credit tracking (E3-S3), automatic keepalive, transparent `Resume` on drop within
  grace (surfaced as an event, never silent).
- `#![forbid(unsafe_code)]` continues from the workspace lints.

#### X3-E9-S2 — Sync façade
**AC**
- The existing `BrowseSession` gains v3 methods via a blocking façade over the
  async client so the `xs` CLI and f2 keep a synchronous API.

#### X3-E9-S3 — CLI verbs
**AC**
- `xs ls|stat|cat|get|put|mkdir|rm|rmdir|mv|df|watch xsync://[user@]host[:port]/export/path`
  for debugging and scripting; `--json` for each; exit codes documented and tested
  like the sync path's.
- `xs mounts` shows sessions on a daemon (operator view, E12-S3).

#### X3-E9-S4 — Error mapping
**AC**
- `xsync_client::Error` maps to `std::io::ErrorKind` and exposes the wire code and
  `platform_errno`; a table in docs is generated from the enum so it cannot drift.

#### X3-E9-S5 — Progress and events
**AC**
- Range transfers emit the existing `progress-json v1` event vocabulary extended
  with `transfer_id`, `bytes_done/total`, `rate`, so a GUI queue can subscribe
  without polling.

#### X3-E9-S6 — Conformance kit
**AC**
- An in-process test server (`xsync_client::testing::Server`) with fault injection
  (drop connection after N frames, delay, deny) that consumers use in their own
  suites — the pattern Kestrel's `start_xs_probe_server` established.

### E10 — Security hardening

#### X3-E10-S1 — Confinement with handles
**AC**
- Every path operation is confined to the export root (existing rule) and every
  handle operation is validated against the handle table, never by re-walking a
  path; `openat`-relative operations from a root descriptor on Unix; `RESOLVE_BENEATH`
  where available.
- `Rename`/`Link` refuse to move a file across export roots even when both are on
  one filesystem.

#### X3-E10-S2 — Resource and abuse limits
**AC**
- Caps from E1-S5 enforced; unauthenticated connections have a handshake deadline
  (default 10 s) and a small pre-auth frame budget; per-address connection rate limit.

#### X3-E10-S3 — Audit log
**AC**
- Opt-in JSONL audit (`principal`, `mapped uid`, `export`, `op`, `path`, `result`,
  `bytes`) reusing `faillog.rs` writer discipline; never logs file contents or
  credentials.

#### X3-E10-S4 — Fuzzing and vectors
**AC**
- `cargo fuzz` targets for the v3 decoder and the handle/credit state machines run
  in CI (extends existing `fuzz/`); malformed vectors added to
  `protocol-v2-vectors/` (or a sibling `protocol-v3-vectors/`).

#### X3-E10-S5 — Threat model
**AC**
- `docs/threat-model.md` covering: malicious client, malicious server (client
  library defends against oversize/unsolicited frames), on-path attacker (TLS/SSH),
  local users on the server host, and the privilege model below.

#### X3-E10-S6 — Privilege model
**AC**
- Default: daemon runs as a dedicated unprivileged user; exports it cannot read are
  load errors; identity mapping is *evaluated* (E2-S4).
- Optional `privileged = true`: daemon runs as root, forks a worker per session that
  drops to the mapped identity before touching the filesystem; workers communicate
  over a socketpair; the root process never performs file I/O.

### E11 — Protocol governance and compatibility

#### X3-E11-S1 — v3 freeze — **Done (Phase 1 table)**

*Landed 2026-09-03 on xsync branch `v3`.*

**AC**
- [x] `protocol.md` gains a v3 table (types from 42), the v3 error table, the feature
  bitmap, and bounds for every field; `v2handshake.md` documents `CAP_FS_V3`.
- [x] Byte-exact vectors, codec tests with malformed coverage, generated compatibility
  matrix with rows for `xsync v3` and `f2 v2`.
- [ ] Matrix rows for `Kestrel` and `Excalibur`. **Deferred:** neither implements v3
  yet, so a row would describe an intention rather than a result. They land with
  Excalibur M1 (`plan.md` E7-S2).

**Results**

- `protocol.md` "v3 message table" freezes 21 types (42–122) plus the three v2 control
  types (18–20) reused unchanged, with reserved ranges named for every later phase so a
  future story adds types without renumbering.
- Frozen alongside the types: the `Attrs` record (presence bitmap + fixed part +
  optional blocks in bit order), the `attr_mask`, the 26-code error table,
  `Open.flags` and their consistency rules, `MountInfo.supports`, and the rule that
  `effective_writable` and `reason` must agree.
- `crates/xsync-core/src/protocol_v3.rs`: fail-closed codec — unknown type, unknown flag
  or presence bit, out-of-range length, inconsistent field pair, or trailing byte is an
  error, and no decode failure is ever a reason to fall back to an older grammar.
- `protocol-v3-vectors/payload-v1.tsv`: 22 valid + 12 malformed vectors, **generated
  from the document** by `scripts/generate-v3-vectors.py` — an independent Python
  implementation of the field layout, so the corpus cross-checks the spec against the
  codec rather than the codec testing itself.
- `docs/compatibility-matrix.md` is generated from both corpora and carries a digest for
  each.
- Two design changes made *during* the freeze rather than after it (both recorded in
  §7): requests carry an `attr_mask u32` instead of `want_names` / `want_attrs`
  booleans, and per-request deadlines were deferred to a session-level control.

**Next steps**

- **This is a freeze candidate, not a hard freeze.** Per `docs/protocol-ownership.md` a
  type is contractually frozen once merged *and* imported by a second consumer; nothing
  has imported the vectors yet, and the branch is uncommitted. Hold the hard freeze until
  Excalibur M1 has actually driven the table — `plan.md` already expects E9-S1's method
  list to change.
- f2 needs no action: it never advertises `CAP_FS_V3` and so never receives a v3 frame.
  The outstanding `CAP_BROWSE_META` f2-vector blocker is unrelated and still open.
- Add the Kestrel and Excalibur matrix rows when they implement v3.

#### X3-E11-S2 — Downgrade behaviour
**AC**
- A v3 client to a v2 server: browse v2 only; `Fs` trait reports `random_access =
  false`, and consumers degrade (Excalibur shows "reduced mode — upgrade the remote
  xsync"); never a hard failure.
- A v2 client to a v3 server: identical to today.

#### X3-E11-S3 — Consumer coordination
**AC**
- f2 and Kestrel maintainers review the freeze; the release that first advertises
  `CAP_FS_V3` names the consumer status per `docs/protocol-ownership.md`, including
  the outstanding `CAP_BROWSE_META` f2-vector blocker.

### E12 — Observability and operations

#### X3-E12-S1 — Metrics
**AC**
- `xs serve --metrics <addr>` exposes Prometheus text: sessions, requests by type,
  bytes in/out, errors by code, open handles, locks, watches, per-export.

#### X3-E12-S2 — Logs
**AC**
- Structured JSONL via `--log-json` (exists) extended with session id and
  principal; log levels configurable; no payloads.

#### X3-E12-S3 — Operator commands
**AC**
- `xs mounts`, `xs sessions --kill <id>`, `xs ping host`, `xs serve --check-config`.

#### X3-E12-S4 — Packaging
**AC**
- Release artifacts for the daemon on macOS and Linux (the `DEPLOYMENT.md` gap),
  `brew` formula and `.deb` with service units; `xs bootstrap` continues to stage
  the binary over SSH for the SSH-transport case.

### E13 — OS mount adapter (stretch)

*Optional. Everything above makes xsync usable **like** NFS/SMB from an application;
this epic makes it mountable **by the OS**, which is what many people mean by
"network mount".*

#### X3-E13-S1 — FUSE client
**AC**
- `xs mount xsync://host/export /mnt/point` implemented over `xsync-client` with the
  `fuser` crate on Linux and macFUSE (or FSKit when stable) on macOS; maps E4/E5/E6
  1:1; honours leases for the kernel attribute cache.
- `umount` cleanly closes the session; a dropped link within grace is transparent.

#### X3-E13-S2 — Loopback NFS gateway
**AC**
- Alternative for hosts without FUSE: `xs nfs-gateway` serves NFSv3 on
  `127.0.0.1` translating to xsync v3, so the OS's own NFS client mounts it with no
  kernel extension. Documented trade-offs (no locks beyond NLM, AUTH_SYS only).

#### X3-E13-S3 — Windows (WinFsp)
**AC**
- Deferred; recorded with the exact "not supported" refusal in the matrix.

---

## 4. Wire additions

**Phase 1 is frozen** (2026-09-03) in `../xsync/protocol.md` "v3 message table", with
the codec in `xsync-core::protocol_v3` and byte-exact vectors in
`../xsync/protocol-v3-vectors/`. Selection and the `Features` exchange (E3-S6) are
implemented too: a `Role::Session` peer advertises `CAP_FS_V3`, and
`probe_fs_session` / `FsSession` drive a v3 session whose filesystem verbs answer
`EOPNOTSUPP` until their handlers land. Everything else in this section is a *proposal*
for later phases; those numbers are reserved and assigned for real through
`docs/protocol-ownership.md`.

**Handshake:** `CAP_FS_V3 = 1 << 7` (frozen). Envelope version byte `3` after
selection. v3 is selected iff both peers advertise `CAP_VERSION_NEGOTIATION` and
`CAP_FS_V3`; otherwise the v2 rule applies.

**Feature bitmap (`Features` 42 / `FeaturesAck` 43, frozen):** `LOCKS`, `LEASES`,
`SHARE_MODES`, `NOTIFY`, `NOTIFY_POLLING`, `XATTR`, `SPARSE`, `COMPOUND`,
`STAGE_RESUME`, `ACCESS`, `OWNER_NAMES` (bits 0–10). The negotiated set is the
intersection; unknown bits are ignored.

| Range | Group | Status | Messages |
|---|---|---|---|
| 18–20 | Shared control | frozen (v2 layout) | `CancelRequest`, `Keepalive`, `KeepaliveAck` |
| 42–43 | Negotiation | **frozen** | `Features`, `FeaturesAck` |
| 44–49 | Auth & discovery | reserved (Phase 2) | `AuthStart`, `AuthChallenge`, `AuthResponse`, `AuthResult(SecurityInfo)`, `ListExports`/`ExportsList` |
| 50–51 | Mount | **frozen** | `Mount`/`MountInfo` |
| 52–55 | Session | reserved (Phase 3) | `Resume`/`ResumeResult`, `Unmount`, `MountChanged`* |
| 56–63 | Handles & I/O | **frozen** | `Open`/`Opened`, `Close`, `Read`/`ReadData`, `Write`/`WriteAck`, `Flush` |
| 64–69 | Handles & I/O | reserved (Phases 3–4) | `SetSize`, `Allocate`, `Seek`/`SeekResult`, `Advise`, `HandleState` |
| 70–79 | Staging | reserved (Phase 3) | `StageOpen`/`StageOpened`, `StageWrite`/`StageAck`, `StageStatus`/`StageRanges`, `StageCommit`/`StageResult`, `StageAbort` |
| 80–85 | Namespace | **frozen** | `Stat`/`Attrs`, `ReadDir`/`DirPage`, `StatFs`/`FsInfo` |
| 86–99 | Namespace | reserved (Phases 3–4) | `Rename3`, `Unlink`, `Rmdir`, `Mkdir3`, `Symlink`, `Link`, `Chown`, `SetTimes`, `Access`/`AccessResult`, `ListXattr`, `GetXattr`, `SetXattr`, `RemoveXattr`, `XattrResult` |
| 100–109 | Locks | reserved (Phase 3–4) | `Lock`/`LockResult`, `Unlock`, `TestLock`/`TestLockResult`, `LeaseBreak`*, `LeaseAck` |
| 110–115 | Notify | reserved (Phase 4) | `Watch`/`Watched`, `Unwatch`, `WatchEvent`*, `WatchOverflow`* |
| 116–119 | Compound | reserved (Phase 4) | `Compound`/`CompoundResult` |
| 120 | Control | reserved (Phase 2) | `Shutdown`* |
| 121–122 | Control | **frozen** | `Error`, `Done` |

`*` = server-initiated notification, `related request ID = 0`.

Frozen in Phase 1 alongside the types: the `Attrs` record (presence bitmap + fixed
part + optional blocks in bit order), the `attr_mask` carried by `Open`, `Stat` and
`ReadDir` (same bit numbering; unknown *mask* bits ignored, unknown *presence* bits
rejected), the 26-entry error-code table (`1 ENOENT` … `26 ELEASEBROKEN`),
`Open.flags` (`READ`, `WRITE`, `CREATE`, `EXCL`, `TRUNC`, `APPEND`, `NOFOLLOW`,
`DIRECTORY`) and their consistency rules, `MountInfo.supports` bits, and the rule
that `MountInfo.effective_writable` and `reason` must agree. Field
encodings reuse `protocol.md`'s rules (little-endian, `u32`-prefixed byte strings,
bounded counts, 16 MiB payload cap, 8 MiB data cap, fail-closed on trailing bytes).

---

## 5. Sequencing

Ordered so that Excalibur's xsync backend (`plan.md` Epic 7) can start as early as
possible, and so every phase is independently shippable and negotiable.

**Phase 1 is built before Excalibur's first real milestone** (`plan.md` M-1 → M1):
xsync is Excalibur's primary backend, so `xsync-client::Fs` (E9-S1) and Excalibur's
`ShareBackend` are co-designed rather than one adapting to the other. Expect E9-S1's
method list to be revised against what the GUI actually needs once M1 is running.

| Phase | Stories | Unlocks |
|---|---|---|
| **1. Random access over SSH** | ~~E11-S1 (table)~~, ~~E3-S6~~, ~~E3-S7~~, ~~E3-S1~~, **E1-S4 next**, E4-S1–S3, E4-S8, E5-S1–S3, E1-S4 (writability over `--server` with a `--read-only` flag), E9-S1, E9-S4, E9-S6, E10-S1, E10-S4 | Excalibur can browse, preview, stream media, edit-in-place and show RW/RO + capacity against `ssh host xs --server`. **This is the minimum Excalibur dependency.** |
| **2. Daemon, TLS, identity** | E1-S1–S3, E1-S5, E2-S1, E2-S3–S5, E10-S2, E10-S6, E12-S1–S3 | Connect without SSH; exports; principals; the "New Connection" dialog's xsync tab is complete. |
| **3. Durability and coherence** | E3-S2, E3-S3, E3-S5, E4-S6, E4-S7, E6-S1, E6-S4, E6-S5, E5-S4, E5-S7 | Resumable uploads that survive drops; locks for edit-in-place; complete mutation set. |
| **4. Live views and speed** | E7, E6-S2, E8-S1–S5, E5-S2 cursor fix, E5-S5, E4-S4, E4-S5 | Auto-refreshing listings, leases, compound open-read, sparse. |
| **5. Ops and release** | E1-S2 hardening, E12-S4, E10-S3, E10-S5, E11-S2–S3 | Packaged daemon; consumer coordination; release. |
| **6. Stretch** | E13, E1-S6, E6-S3, E8-S6 | OS mounts; Windows server; share modes. |

---

## 6. Explicitly out of scope for v3

- **Delta transfer / rolling checksums** — still gated on the README §14 cost
  model; orthogonal to filesystem semantics.
- **Remote → remote sync** — unchanged.
- **A persistent index / change journal for sync** — E7 gives sync a change *source*
  but the index itself is the "XL" item in README §14 and stays there.
- **ACL semantics beyond mode bits** — `Access` (E5-S7) reports the effective answer;
  reading/writing ACL entries is a later epic.
- **Replacing browse v2** — it stays frozen and supported for f2.

---

## 7. Decisions

### Taken at the Phase 1 freeze (2026-09-03)

1. **Optional attributes are requested by a mask, not by booleans.** `Open`, `Stat`
   and `ReadDir` carry `attr_mask u32` using the presence bitmap's numbering, rather
   than the `want_names` / `want_attrs` booleans an earlier draft had. A boolean
   freezes one question; the mask freezes the *shape* of the question, so a later
   phase adds an optional block without a new message type. Unknown mask bits are
   ignored (capability-bit rule) while unknown presence bits are rejected (a decoder
   cannot skip a block of unknown length).
2. **Per-request deadlines are a session-level control, not a request field.** E3-S4's
   optional deadline is deliberately *not* in the Phase 1 `Read`/`Write`/`ReadDir`
   payloads: it would cost four bytes on every hot-path request to express something
   almost every client sets once. It lands in Phase 3 as a session control message
   alongside credits (E3-S3). The consequence, accepted: a Phase 1 client cannot give
   one slow request a shorter deadline than the session default, and must `Cancel`
   instead.
3. **`Compound` does not land in Phase 1.** Types 116–119 are reserved. It saves one
   round trip per preview, but a nested decoder needs its own fuzzing, and E8-S2 can
   land without renumbering anything.
4. **Phase 1 frames are uncompressed.** The envelope's zstd flag is reserved for
   E8-S4; a compressed v3 frame is rejected today.

### Still open

5. **Binary shape:** `xs serve` subcommand vs. a separate `xsd` binary (packaging
   prefers a separate binary with a smaller dependency tree; the codebase prefers
   one). Needed for Phase 2.
6. **Default port** and IANA-style registration in docs. Needed for Phase 2.
7. **Grace window default** (60 s proposed) vs. Samba's durable-handle default
   (16 min) — trade-off in E6-S5. Needed for Phase 3.
8. **Windows non-UTF-8 policy** — this document proposes *reject* (`EILSEQ` is in the
   frozen error table for it); `deferred.md` still leaves it open.
