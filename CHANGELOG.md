# Changelog

Notable changes per release. Every released version needs an entry here: the
release workflow refuses to publish a tag whose version is missing.

Each entry states the **protocol version** and whether it is **wire-compatible**
with the previous release, because that is the question an operator upgrading
one end of a transfer actually has.

## [Unreleased]

### Added
- Protocol v3 `Mount`: a session attaches to the `xs --server` root and receives
  its writability, the reason a write is refused, case/normalization semantics,
  name and path limits, and a `supports` bitmap. `xs --server --read-only` serves
  an export read-only; write-class requests are then refused with `EROFS` before
  the filesystem is touched. Every other verb requires a completed mount.
- Protocol v3 concurrency: a v3 session dispatches requests to a bounded worker
  pool and answers them out of order, while requests naming the same handle stay
  in send order. `Features`, `Keepalive` and `Cancel` are answered without waiting
  for the pool. Past the in-flight cap a request is refused with `ELIMIT` rather
  than stalling the reader; `Server::with_fs_limits` tunes both bounds.
- Protocol v3 negotiation: a `Role::Session` peer advertises `CAP_FS_V3`,
  `negotiate_protocol_version` selects 3/2/1, and a selected v3 session opens
  with a `Features` exchange whose intersection gates every optional message
  group. New client entry points `probe_fs_session` and `FsSession`, and a new
  `ProbeStatus::ReadyV3`. `probe_session` is unchanged and still never selects
  v3, so an existing browse consumer keeps its behaviour against a v3 server.
  Filesystem verbs answer `EOPNOTSUPP` until their handlers land.
  `protocol-negotiated` gains `fs_v3_available` and `fs_v3_features`.
- Protocol v3 Phase 1 freeze: the filesystem message table (types 42–122,
  reserved ranges included), the `Attrs` record, the frozen error-code table,
  `CAP_FS_V3`, the `xsync-core::protocol_v3` fail-closed codec, and
  `protocol-v3-vectors/`. No implementation advertises the bit yet; selection
  lands with `xsyncv3.md` E3-S6. See `protocol.md` "v3 message table".
- `--bootstrap=once|persist` uploads a checksum-verified binary to a remote that
  has none (D5.2). See `docs/remote-bootstrap.md`.
- `--log-json FILE` writes structured failure records from both the client and
  the remote server. See `docs/failure-log-v1.md`.
- `xs --version` reports the commit, build date, target triple, protocol
  versions, and enabled features; `-V` gives the short form.
- Supply-chain gates in CI (`cargo deny`, `cargo audit`); policy in `deny.toml`,
  audited result in `docs/supply-chain.md`.
- `LICENSE-MIT` and `LICENSE-APACHE`, which the crate declared but did not ship.

### Fixed
- Windows builds at all: seven `WirePath` call sites did not compile there.
- A trailing backslash is now a path separator on Windows. `xs C:\data\ C:\backup`
  silently copied the directory *into* the destination instead of syncing its
  contents, and reported success.
- Unchanged files are skipped across unlike filesystems. Modification times were
  compared for exact equality, but NTFS stores 100 ns ticks where APFS and ext4
  store nanoseconds, so every macOS-to-Windows sync re-transferred everything.
- `--checksum` classifies by content *instead of* size+mtime, as documented,
  rather than in addition to it.
- A stock Windows remote works over SSH with no configuration: the server
  command is emitted in `cmd.exe` syntax when the remote needs it.

**Protocol:** wire v1, browse v2. **Wire compatibility:** unchanged from v0.1.0.

## [0.1.0]

First tagged release.

**Protocol:** wire v1, browse v2. **Wire compatibility:** initial release; no
previous version to compare against.
