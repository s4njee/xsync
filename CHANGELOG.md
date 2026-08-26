# Changelog

Notable changes per release. Every released version needs an entry here: the
release workflow refuses to publish a tag whose version is missing.

Each entry states the **protocol version** and whether it is **wire-compatible**
with the previous release, because that is the question an operator upgrading
one end of a transfer actually has.

## [Unreleased]

### Added
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
