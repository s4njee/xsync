# xsync deployment — Epics, Stories & Acceptance Criteria

Roadmap from "a binary that builds on the author's Mac" to "a signed, packaged binary a
stranger can install on Windows, Linux, or macOS and run."

Companion documents: [plan.md](plan.md) for the v1 design, [tasks.md](tasks.md) for engine
stories, [TUNING.md](TUNING.md) and [TUNING-TASKS.md](TUNING-TASKS.md) for v2 performance
work. This file is orthogonal to tuning: it can proceed in parallel.

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

---

## Where the project actually stands

Measured against the working tree, not assumed:

| Fact | State |
|---|---|
| CI | **None.** No `.github/`, no pipeline of any kind |
| macOS (aarch64) | Builds, 124 tests pass, clippy clean at `-D warnings` |
| Linux | Builds natively on `mars` (Arch, x86_64); never built in CI |
| **Windows** | Compiles (`cargo check` / `clippy -D warnings` for `x86_64-pc-windows-msvc`). Linking/tests need MSVC (`link.exe`); see D0.1 |
| Compression on Windows | zstd enabled for compression and decompression |
| Code signing | None on any platform |
| Packaging | None. No tarball, installer, package, or tap |
| Service integration | None. No systemd unit, launchd plist, or Windows service |
| Repository URL | `https://example.invalid/xsync` placeholder in `Cargo.toml` |
| Release profile | Already good: `lto = true`, `codegen-units = 1`, `strip = true` |
| Supply chain | `unsafe_code = "deny"` workspace-wide; no audit tooling wired up |
| Repo hygiene | 715 MB / 22,650 untracked files under `benches/results/tuning/` |

The single most important consequence: **there is no CI.** Every story in D1 exists so that
platform-specific build and test regressions cannot recur.

---

## Epic D0 — Make it build everywhere

Blocking. Nothing downstream matters until all three platforms compile.

### Story D0.1 — Fix the Windows build
- [x] `cargo check -p xsync --target x86_64-pc-windows-msvc` passed after fixing the cfg-gated build paths.

**AC**
- The break is an arity mismatch behind a `cfg`: `compress_zstd` at
  `crates/xsync-core/src/protocol.rs:613` takes one argument on Windows, while the caller at
  `:435` passes two (`payload`, `level`). The Windows and non-Windows signatures must be
  identical.
- The unused-variable warning at `crates/xsync-core/src/journal.rs:372` (`sync_parent`) is
  resolved rather than silenced, or explicitly justified.
- `cargo check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` all pass for
  `x86_64-pc-windows-msvc`.
- A grep for `cfg(target_os = "windows")` and `cfg(windows)` confirms every gated function
  has a signature-compatible counterpart. There are currently 10 such sites in `sink.rs`,
  11 in `rsync.rs`, 9 in `scanner.rs`, 4 each in `source.rs` and `protocol.rs`.

### Story D0.2 — Decide the Windows compression story
- [x] `zstd` is enabled on all platforms, including Windows.

**AC**
- Decision: enable `zstd` on all platforms and delete the Windows gate. The project targets
  `x86_64-pc-windows-msvc`, whose supported MSVC build environment supplies the native C
  toolchain required by `zstd-sys`; avoiding that toolchain is not a release constraint.
- This keeps compression and decompression wire-compatible across peers and makes the
  platform-specific `CompressionUnavailable` path unnecessary.
- `ProtocolError::CompressionUnavailable` was removed because zstd is now available in normal
  operation on every supported target.

### Story D0.3 — Compression negotiation degrades safely
- [x] A peer without compression support talking to a peer with compression enabled
  negotiates uncompressed frames safely instead of failing the transfer.

**AC**
- The handshake already carries a `compression` field; the negotiated mode is the
  intersection of both peers' capabilities, chosen before any data frame is sent.
- Integration test: a peer built without compression support interoperates with one built
  with it, in both directions, and the transfer succeeds uncompressed.
- The `done` event and `--progress-json` report the negotiated algorithm and the reason it
  was chosen, so a silent downgrade is observable.

### Story D0.4 — Define the target matrix
- [x] The supported, best-effort, and excluded target triples are defined in
  [docs/TARGET-MATRIX.md](docs/TARGET-MATRIX.md), with Linux builder images for the
  glibc floor and musl bootstrap binaries.

**AC**
- Tier 1 (built, tested, signed, released every tag): `aarch64-apple-darwin`,
  `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-pc-windows-msvc`.
- Tier 2 (built and tested, not necessarily signed): `aarch64-pc-windows-msvc`, musl targets
  for static Linux binaries.
- The glibc floor for Linux Tier 1 builds is stated explicitly and enforced by the builder
  image, so binaries do not fail on older distributions.
- A musl target is provided for "works on any Linux" distribution, since that is the natural
  fit for the remote-bootstrap case in D5.

---

## Epic D1 — Continuous integration

The project has none. This epic is what makes every other guarantee in this file durable.

### Story D1.1 — Build, test, and lint on every Tier 1 target
- [x] CI runs on push and pull request via `.github/workflows/ci.yml`.

**AC**
- Matrix covers macOS (aarch64 and x86_64), Linux (glibc and musl, x86_64 and aarch64), and
  Windows (x86_64), each running `cargo build`, `cargo test`, and
  `cargo clippy --all-targets -- -D warnings`.
- MSRV is pinned to the declared `rust-version = "1.88"` and verified by a dedicated job, so
  the manifest cannot drift from reality.
- `cargo fmt --check` gates formatting.
- A red build blocks merge. The Windows job must be able to fail — it currently would.

### Story D1.2 — Cross-platform integration coverage
- [~] The integration harness now has a Windows PowerShell-backed `--rsh` helper; full
  cross-platform CI execution and filesystem edge-case coverage are in progress.

**AC**
- `crates/xsync/tests/server_integration.rs` passes on all three operating systems, not just
  macOS.
- Path handling is exercised on Windows specifically: drive letters, backslash separators,
  case-insensitive collisions, and paths beyond 260 characters.
- Non-UTF-8 filenames are exercised on Unix and the Windows behaviour is documented, since
  the protocol carries paths as raw bytes.
- A cross-OS transfer test (Linux receiver, Windows sender or vice versa) runs in CI, since
  D0.3's interop bug would have been caught by exactly this.

### Story D1.3 — Dependency and supply-chain gates
- [ ] The workspace already denies `unsafe_code`; nothing verifies the dependency tree.

**AC**
- `cargo audit` (advisories) and `cargo deny` (licences, duplicate versions, banned crates)
  run in CI and fail on new findings.
- `Cargo.lock` is committed and CI builds `--locked`.
- The licence set across all transitive dependencies is compatible with the declared
  `MIT OR Apache-2.0`, and the result is checked in.

---

## Epic D2 — Reproducible release artifacts

### Story D2.1 — Version stamping and provenance
- [ ] A user reporting a bug must be able to say exactly what they ran.

**AC**
- `xsync --version` reports the semantic version, the git commit, the build date, and the
  target triple.
- The version comes from a single source of truth; a tag and the binary can never disagree.
- `--version --verbose` (or equivalent) additionally reports enabled features — notably
  whether compression is available — so D0.2's outcome is visible in the field.

### Story D2.2 — Release build and artifact set
- [ ] One command, or one tag, produces every artifact.

**AC**
- Tagging `vX.Y.Z` produces binaries for every Tier 1 target, named consistently
  (`xsync-<version>-<target>.tar.gz`, `.zip` for Windows).
- Each archive contains the binary, `LICENSE-MIT`, `LICENSE-APACHE`, and a `README`.
- `SHA256SUMS` is published alongside, and build provenance attestation is generated.
- Builds are reproducible: two runs from the same commit produce byte-identical binaries, or
  the sources of nondeterminism are documented.

### Story D2.3 — Release automation
- [ ] Cutting a release must not be a manual ritual.

**AC**
- A tag triggers build, sign, checksum, and publish without manual steps beyond approving
  the release.
- A changelog is generated or enforced, and the release notes state the protocol version and
  whether it is wire-compatible with the previous release.
- A dry-run mode allows validating the pipeline without publishing.

---

## Epic D3 — Signing and platform trust

Unsigned binaries are actively hostile to install on macOS and Windows. This epic is what
separates "a binary exists" from "a stranger can run it."

### Story D3.1 — macOS signing and notarization
- [ ] An unsigned, un-notarized binary is blocked by Gatekeeper on download.

**AC**
- Binaries are signed with a Developer ID Application certificate and hardened runtime, then
  submitted for notarization and stapled.
- `spctl -a -vvv -t install` accepts the artifact on a clean machine.
- Both architectures are signed; a universal binary is produced if it simplifies
  distribution.
- Signing identities live in CI secrets, never in the repository, and the release job fails
  closed if they are absent.

### Story D3.2 — Windows Authenticode signing
- [ ] SmartScreen penalises unsigned executables heavily.

**AC**
- The `.exe` is Authenticode-signed with a timestamp so signatures survive certificate
  expiry.
- If an EV certificate is not available, the reputation implications are documented so
  expectations are set rather than discovered.
- The installer from D4.4, if any, is signed as well as the binary.

### Story D3.3 — Linux integrity
- [ ] Linux has no equivalent signing requirement, but distribution does.

**AC**
- Package repositories, if published, are signed with a documented key and rotation policy.
- Detached signatures accompany tarballs so `SHA256SUMS` can itself be verified.
- The verification procedure is documented in a form a user will actually follow.

---

## Epic D4 — Packaging and distribution

Ordered by reach per unit of effort.

### Story D4.1 — Direct download and install script
- [ ] The lowest-friction path, and the one every other channel falls back to.

**AC**
- A documented one-liner installs the correct artifact for the detected platform and
  architecture, verifies its checksum, and places it on `PATH`.
- The script refuses to proceed on checksum mismatch and prints the manual steps.
- It works without root by installing to a user-local prefix, and says where it put things.

### Story D4.2 — Homebrew
- [ ] Covers macOS and Linux developer machines.

**AC**
- `brew install xsync` works from a tap; formula updates are automated on release.
- Both `aarch64` and `x86_64` macOS are covered by bottles.

### Story D4.3 — Linux packages
- [ ] `.deb` and `.rpm` for the distributions the target users run.

**AC**
- Packages install the binary, man page, and shell completions to standard locations.
- Package metadata declares the glibc floor from D0.4, so installation fails cleanly rather
  than the binary failing at runtime.
- The systemd unit from D6.1 is included but not enabled by default.

### Story D4.4 — Windows distribution
- [ ] The channel Windows users expect.

**AC**
- Published to `winget` and/or `scoop`; an MSI is provided if the service integration in
  D6.3 requires one.
- Install adds `xsync` to `PATH` for the installing user.
- Uninstall removes everything the installer added, verified on a clean VM.

### Story D4.5 — crates.io
- [ ] `cargo install xsync` and library reuse of `xsync-core`.

**AC**
- The placeholder `repository = "https://example.invalid/xsync"` in `Cargo.toml` is replaced
  with the real URL — publishing is impossible until it is.
- `xsync-core` publishes with usable documentation, since the plan positions it as the
  library a GUI would embed.
- `cargo install xsync` produces a working binary on all Tier 1 platforms.

---

## Epic D5 — Getting xsync onto the far end

This epic is specific to xsync and is easy to overlook. rsync's real advantage is that it is
*already installed everywhere*. A sync tool that requires itself on both ends has a
distribution problem that no amount of packaging solves on its own.

### Story D5.1 — Diagnose and guide when the remote is missing
- [ ] The existing error is a good start: "xsync not found on remote host — install it or
  check PATH".

**AC**
- The message names the remote host, the expected binary, and the exact install command for
  that platform where it can be detected.
- The fallback to the native rsync transport (Story 4.5) is offered explicitly, with its
  reduced guarantees stated, rather than being silently substituted.
- Behaviour is covered by an integration test using the `--rsh` harness.

### Story D5.2 — Remote bootstrap
- [ ] Optionally push a matching binary to a host that lacks one.

**AC**
- An explicit, opt-in flag copies a verified binary for the remote's detected platform to a
  user-writable location and uses it for the session.
- The binary's checksum is verified on the remote before execution.
- It never installs system-wide, never requires root, and cleans up unless told to persist.
- Refuses on architecture or libc mismatch with a clear message rather than shipping
  something that will not run — this is where the musl target from D0.4 earns its place.

### Story D5.3 — Version and protocol compatibility policy
- [ ] Two hosts will not upgrade simultaneously.

**AC**
- The existing version-mismatch error (`xsync version mismatch: local vX / remote vY`) is
  extended to distinguish protocol incompatibility from a mere version difference.
- The support policy states which version skew is allowed, and CI tests at least one skewed
  pair.
- `protocol.md`'s rule that new message types require a version bump is reflected in the
  release-notes template.

---

## Epic D6 — Service integration

The original brief calls for a systemd service on Linux, a system-tray server on Windows,
and launchd on macOS. plan.md defers these to v2; they are listed here because they are
deployment surface, and because packaging (D4) must know whether it is shipping them.

### Story D6.1 — systemd unit
- [ ] Run the daemon as a managed service.

**AC**
- A unit file with a hardened sandbox (`ProtectSystem`, `NoNewPrivileges`, a dedicated
  user), installed but not enabled by default.
- Both system and user-level units are supported, since most sync targets are per-user data.
- Logs go to the journal, and log level is configurable without editing the unit.

### Story D6.2 — launchd agent
- [ ] macOS equivalent.

**AC**
- A `LaunchAgent` plist for per-user operation, loadable with `launchctl` and surviving
  reboot.
- Requests only the permissions it needs; Full Disk Access requirements are documented,
  since a sync tool reading a home directory will hit TCC prompts.
- Uninstall fully unloads and removes the agent.

### Story D6.3 — Windows service and tray
- [ ] The most differentiated piece — cwRsync and WSL workarounds are the incumbent, and are
  bad.

**AC**
- A Windows service hosts the engine; a separate tray application provides status and
  control, since a service cannot present UI directly.
- Service install, start, stop, and uninstall are exercised on a clean VM in CI or a
  documented manual checklist.
- The tray communicates with the service over the local control socket rather than
  duplicating engine logic.

### Story D6.4 — Configuration
- [ ] A daemon needs configuration that a CLI flag cannot carry.

**AC**
- A documented config file format with a stated search path per platform.
- Precedence between config file, environment, and CLI flags is defined and tested.
- A malformed config fails at startup with a precise error, never with partial application.

---

## Epic D7 — Install experience

### Story D7.1 — Shell completions and manual page
- [ ] Expected of any serious CLI, and cheap given clap.

**AC**
- Completions generated for bash, zsh, fish, and PowerShell, installed by the packages in D4.
- A man page is generated from the same source as `--help`, so they cannot drift.

### Story D7.2 — Uninstall and state cleanup
- [ ] xsync leaves state outside its binary and must be able to clean up after itself.

**AC**
- Documented and implemented removal of the hash cache (`~/.cache/xsync/hashes.redb`), resume
  journals (`$TMPDIR/xsync-resume-*`), and any stale `.xsync.tmp.*` staging files.
- A command reports what state exists and where, before removing anything.
- Uninstalling via any D4 channel leaves no orphaned services or agents.

### Story D7.3 — First-run documentation
- [ ] Depends on Story 8.2's README.

**AC**
- Install, first sync, and remote sync are demonstrable from the README alone on each
  platform.
- Platform-specific caveats are stated up front: macOS TCC prompts, Windows path length and
  case sensitivity, and the v1 limitations already listed in plan.md.
- No performance claim appears without its corpus, route, and baseline, per the Story 8.2
  acceptance criteria.

---

## Epic D8 — Release readiness

### Story D8.1 — Repository hygiene
- [ ] The tree is not currently in a shippable state.

**AC**
- 715 MB and 22,650 files of benchmark output under `benches/results/tuning/` are either
  ignored, moved out of the repository, or reduced to the reports alone. Destination trees
  copied from real corpora must never be committed.
- `.gitignore` covers benchmark scratch, corpora, and `__pycache__`.
- `Cargo.toml`'s placeholder repository URL is replaced (also required by D4.5).
- No absolute paths from the author's machine appear in committed files.

### Story D8.2 — Security posture statement
- [ ] Users running a binary that spawns remote processes deserve an explicit statement.

**AC**
- Documented: remote server trust model, destination path containment, symlink handling,
  protocol allocation limits, and temp/journal cleanup — matching the Story 8.2 criteria.
- The `unsafe_code = "deny"` posture is stated, along with any dependency that uses unsafe on
  the project's behalf.
- A security contact and disclosure policy exist before the first public release.

### Story D8.3 — Support matrix
- [ ] What is promised, and what is not.

**AC**
- A published table of supported platforms, their tier, glibc floor, and known limitations.
- v1 limitations from plan.md are repeated here: no delta transfer, hardlinks, xattrs, ACLs,
  sparse preservation, ownership, or remote-to-remote.
- Sparse files deserve a prominent warning until TUNING-TASKS Epic T2 lands: a 130 GB sparse
  image currently reads and writes 3.7 TB and cannot complete.

---

## Execution order

**D0 first** — nothing ships while Windows does not compile, and D0.3's interop bug would
corrupt any cross-platform release. **D1 immediately after**, because every guarantee made
later is only as durable as the CI that re-checks it; the Windows rot happened precisely
because nothing was watching.

Then D2 and D3 together (artifacts are not useful unsigned), then D4 in the order of that
epic's stories. D5 can proceed in parallel from D0.4 onward and matters more than it looks:
it is the difference between "install xsync on both machines" and "run xsync."

D6 tracks the v2 daemon work in plan.md and should not gate a first CLI release. D7 and D8
are the final pass before a public tag.

**Minimum shippable release:** D0 complete, D1.1 and D1.2 green, D2.1 and D2.2, D3.1 and
D3.2, D4.1, D5.1, D8.1, and D8.3. Everything else improves reach or experience but is not
required for a binary a stranger can safely install and run.
