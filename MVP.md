# xsync MVP — running `xs` across the home network

Operational guide for using this repository's `xs` binary to move files between
four hosts: **freya** (amd64 Linux), **mars** (amd64 Linux, dual-booted with
Windows), **rpi5** (arm64 Gentoo), and the **M1 Max MacBook Pro** (dev machine
and build host).

Companion documents: [plan.md](plan.md) for the v1 design, [DEPLOYMENT.md](DEPLOYMENT.md)
for the packaging roadmap, [docs/TARGET-MATRIX.md](docs/TARGET-MATRIX.md) for the
supported target contract, [docs/linux-staging.md](docs/linux-staging.md) for the
staging script this guide wraps.

Everything below was exercised against this working tree on 2026-08-25 unless a
line explicitly says otherwise.

---

## 1. What actually works today

The engine is real and the remote transport works. What is missing is
*distribution*: there is no release pipeline, no packages, and no installer, so
every host is provisioned by copying a binary you built yourself.

| Capability | State |
|---|---|
| local → local | Works, with an APFS/reflink directory-clone fast path |
| local → remote (push) | Works over `ssh`, native protocol v1 |
| remote → local (pull) | Works over `ssh`, native protocol v1 |
| remote → remote | **Not supported** — rejected at argument parsing |
| rsync fallback | Push only, and only against **GNU rsync protocol 32** |
| Windows | Compiles and passes CI as a *client*; unusable as an SSH *server* (§7) |
| Packaging / installer | None. Manual staging is the only path (§4) |
| Code signing | None on any platform |

Metadata preserved: mtimes, Unix permission bits, empty directories, and
symlinks as symlinks. **Not** preserved: hardlinks, ownership, ACLs, xattrs,
resource forks, and sparse layout.

Update model is **whole-file**. There is no delta/rsync-style block transfer in
v1 — a one-byte change in a 4 GB file resends 4 GB. Default equality is
type + size + mtime; `--checksum` switches to BLAKE3.

### Health of the tree, as measured

| Tree | `cargo test -p xsync-core -p xsync` |
|---|---|
| `HEAD` (`8ca26cce`) | **143 passed, 0 failed** |
| Working tree (uncommitted v2 work) | **135 passed, 8 failed** |

The 8 failures are a **stale test harness, not an engine defect** — see §10.1.
End-to-end push and pull through a correctly-shaped fake remote shell succeed on
the working tree for small trees, a 20 MiB file, and a 30 MiB file, at both
`--streams 1` and `--streams 4`.

---

## 2. Host matrix

| Host | OS / arch | Rust target triple | Role |
|---|---|---|---|
| mbp | macOS 26, Apple Silicon | `aarch64-apple-darwin` | Build host + endpoint |
| freya | Linux, amd64 | `x86_64-unknown-linux-gnu` | Endpoint |
| mars (Linux) | Linux, amd64 | `x86_64-unknown-linux-gnu` | Endpoint |
| mars (Windows) | Windows, amd64 | `x86_64-pc-windows-msvc` | Client-only endpoint (§7) |
| rpi5 | Gentoo, arm64 | `aarch64-unknown-linux-gnu` | Endpoint |

All four Unix targets are Tier 1 in [docs/TARGET-MATRIX.md](docs/TARGET-MATRIX.md).
The Linux glibc floor is **2.28**.

> **Gentoo note.** Run `ldd --version` on the rpi5 first. A glibc profile takes
> the `-gnu` target above. A musl profile needs `aarch64-unknown-linux-musl`
> instead — see §4.4.

---

## 3. The rule that shapes every command

**One end of every transfer must be local.** `xs freya:/a rpi5:/b` fails with
`remote-to-remote` before doing anything.

Two consequences for a four-host network:

1. **`xs` must be installed on all four hosts**, not just the Mac. Any host can
   be the initiator, and the initiator is always one of the two endpoints.
2. **freya → rpi5 is driven from freya or from rpi5**, never from the Mac:

   ```bash
   ssh freya 'xs /srv/media/ rpi5:/srv/media/'
   ```

   This works because `xs` on freya spawns `ssh rpi5 xs --server /srv/media/`
   itself. It is not remote-to-remote; freya is a local endpoint of its own
   transfer.

---

## 4. Day-1 provisioning

### 4.1 Prerequisites on the Mac

```bash
brew install zig && cargo install cargo-zigbuild
```

`zig` and `cargo-zigbuild` are already present on this machine. Also confirm the
Linux targets are installed:

```bash
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
```

Each Linux host needs only SSH, `sha256sum` (or `shasum`), `mv`, `rm`, `mkdir`,
and an executable filesystem. **No Rust, no compiler, no root, no package
manager.**

### 4.2 Install on the Mac

```bash
cargo install --path crates/xsync
```

That puts `xs` in `~/.cargo/bin/xs`. Confirm:

```bash
xs --version
```

### 4.3 Stage the three Linux hosts

`scripts/stage-linux.sh` cross-builds, uploads under a temporary name, verifies
the SHA-256 on the remote, and only then `mv`s it into place. An interrupted
upload leaves the previous binary untouched. Re-running is safe.

```bash
scripts/stage-linux.sh freya "$HOME/.local/bin/xs" amd64
```

```bash
scripts/stage-linux.sh mars "$HOME/.local/bin/xs" amd64
```

```bash
scripts/stage-linux.sh rpi5 "$HOME/.local/bin/xs" arm64
```

Note that `$HOME` expands on the **Mac**, not on the remote. If the remote
username differs, pass the literal remote path, or use the wrapper that resolves
it for you:

```bash
scripts/deploy-mars.sh mars
```

`deploy-mars.sh [host] [remote-path] [amd64|arm64]` resolves the remote `$HOME`
over SSH first, then delegates to `stage-linux.sh`. It works for any host, not
just mars, despite the name.

### 4.4 If the rpi5 is a musl profile

```bash
rustup target add aarch64-unknown-linux-musl
```

```bash
cargo zigbuild --release --target aarch64-unknown-linux-musl -p xsync
```

Then copy `target/aarch64-unknown-linux-musl/release/xs` to `~/.local/bin/xs` on
the pi yourself — `stage-linux.sh` only maps `amd64`/`arm64` to the two glibc
targets.

### 4.5 If a host reports a GLIBC version error

Pin the floor explicitly; `cargo-zigbuild` accepts a glibc suffix on the triple:

```bash
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.28 -p xsync
```

---

## 5. SSH setup

`xs` uses ordinary OpenSSH. It does not create, require, or modify a
`ControlMaster` socket, and it does not touch host-key checking, authentication,
or agent policy — that is a deliberate design decision recorded in
[docs/browse-connection-model.md](docs/browse-connection-model.md).

Key-based auth is effectively required: `xs` spawns a non-interactive
`ssh host <command>` per transfer and cannot answer a password prompt.

`~/.ssh/config` on every host:

```
Host freya
    HostName freya.local
    User sanjee

Host mars
    HostName mars.local
    User sanjee

Host rpi5
    HostName rpi5.local
    User sanjee

Host *
    ServerAliveInterval 30
    ServerAliveCountMax 6
```

### 5.1 The remote PATH question — check this before anything else

`xs` invokes the far end as `ssh HOST 'xs --server <path>'`. That is a
**non-interactive** SSH session, which on most Linux distributions does *not*
source `~/.bashrc` or `~/.profile`, so `~/.local/bin` may not be on `PATH`.

The uncommitted working tree handles this by prefixing the remote command with
`PATH="$HOME/.local/bin:$PATH"`. **`HEAD` does not.** So the correct answer
depends on which build you deploy. Test it directly:

```bash
ssh freya 'xs --version'
```

If that fails but `ssh freya '~/.local/bin/xs --version'` succeeds, pick one:

- deploy the working-tree build, which adds the PATH prefix itself; or
- stage to a directory already on the default non-interactive PATH:
  `scripts/stage-linux.sh freya /usr/local/bin/xs amd64` (needs write access,
  so `sudo` or a pre-created group-writable `/usr/local/bin`).

Run the `ssh HOST 'xs --version'` check on **all four** hosts. It is the single
most common reason a transfer fails immediately.

---

## 6. Everyday use

Trailing-slash semantics are rsync's: `src/` sends the directory's *contents*,
`src` sends the directory *itself*.

### Push — Mac to a Linux host

```bash
xs ~/Documents/notes/ freya:/srv/backup/notes/
```

### Pull — Linux host to the Mac

```bash
xs rpi5:/srv/media/photos/ ~/Pictures/photos/
```

### Between two Linux hosts (driven from one of them)

```bash
ssh freya 'xs /srv/media/ mars:/srv/media/'
```

### Preview before touching anything

```bash
xs -n --delete ~/Documents/notes/ freya:/srv/backup/notes/
```

Dry run prints one line per planned action (`create`, `update`, `delete`) and
writes nothing. **Always dry-run a `--delete`** — see §10.2 for why the summary
counter cannot be trusted on remote runs.

### Mirror, dropping files the source no longer has

```bash
xs --delete ~/Documents/notes/ freya:/srv/backup/notes/
```

### Skip paths

```bash
xs --exclude 'node_modules/**' --exclude '*.tmp' ~/projects/ mars:/srv/projects/
```

Patterns are globs matched against the path *relative to the root*, using
`globset` semantics (`**` crosses directory boundaries). Excluded directories
are pruned rather than walked. Excludes also disable the directory-clone fast
path.

### Verify content instead of trusting size + mtime

```bash
xs --checksum ~/Documents/notes/ freya:/srv/backup/notes/
```

BLAKE3 digests are cached in `~/.cache/xsync/hashes.redb` (or
`$XDG_CACHE_HOME/xsync/hashes.redb`), so a second `--checksum` run over an
unchanged tree is far cheaper than the first.

### Paranoid mode — re-read every written file and verify its hash

```bash
xs --paranoid ~/important/ mars:/srv/backup/important/
```

### Remote paths

`[user@]host:path` on either side. `~` and `~/sub` expand on the remote:

```bash
xs ~/notes/ mars:~/notes/
```

Windows drive letters (`C:\foo`, `C:/foo`) are always treated as local paths,
never as `host:path`.

### Machine-readable output

```bash
xs --progress-json ~/Documents/notes/ freya:/srv/backup/notes/
```

Emits JSONL to stdout, one object per event, schema in
[docs/progress-json-v1.md](docs/progress-json-v1.md).

### Exit codes

`0` complete · `1` failed · **`23`** partial failure, when some entries
transferred and others did not. Scripts should treat anything non-zero as
needing attention.

---

## 7. mars under Windows

**Use Windows as a client only. Do not make it the remote end of a transfer.**

Windows compiles and is covered by CI, but three things block it as an SSH
server target:

1. **The remote command is POSIX-shell shaped.** `xs` builds
   `PATH="$HOME/.local/bin:$PATH" 'xs' '--server' '/path'` and hands it to
   `ssh HOST '<command>'`. Under the Windows OpenSSH default shell (`cmd.exe`)
   that is not a valid command line. `-e/--rsh` does not help — the POSIX form is
   still appended whenever a host is present. It would work only if the Windows
   sshd default shell were set to a POSIX shell such as Git Bash's `bash.exe`.
2. **The bundled launcher is broken.** `crates/xsync/resources/xsync-server.cmd`
   invokes `%~dp0xsync.exe`, but the binary this workspace produces is named
   **`xs.exe`** (`[[bin]] name = "xs"`). The launcher can never find its target.
3. **The missing-binary probe does not recognise cmd.exe.** The fallback trigger
   matches `xs: command not found` / `xs: not found` / exit 127; cmd.exe says
   `'xs' is not recognized as an internal or external command`, so a missing
   remote binary surfaces as a raw transport error rather than a clean fallback.

**What works today:** Windows *initiating* a transfer to a Linux host. The
remote shell is then Linux `bash`, which parses the command correctly.

Build it on mars itself (cross-compiling `x86_64-pc-windows-msvc` from macOS
needs the MSVC libraries, which are not installed here — and CI produces no
downloadable artifact, since release automation is unimplemented):

1. Install Rust (MSVC toolchain) and Visual Studio Build Tools on mars-Windows.
2. `cargo build --release -p xsync`
3. Copy `target\release\xs.exe` somewhere on `PATH`.
4. Push from Windows: `xs C:\Users\sanjee\Documents\ freya:/srv/backup/docs/`

Also on Windows: filenames that are not valid UTF-8 are unsupported — the v1
scanner represents Windows filenames as UTF-8 protocol paths.

**Recommended MVP posture:** boot mars into Linux for anything where mars is the
destination of a pull or the target of a push from another host. Use
Windows-side `xs` only for ad-hoc pushes out of Windows.

---

## 8. Verification checklist

Run once per host after staging.

```bash
ssh freya 'xs --version'
```

Then a full round trip that proves both directions and byte fidelity:

```bash
mkdir -p /tmp/xs-check/src && head -c 5000000 /dev/urandom > /tmp/xs-check/src/probe.bin
```

```bash
xs /tmp/xs-check/src/ freya:/tmp/xs-check/pushed/ && xs freya:/tmp/xs-check/pushed/ /tmp/xs-check/back/ && shasum -a 256 /tmp/xs-check/src/probe.bin /tmp/xs-check/back/probe.bin
```

The two digests must match. Repeat for `mars` and `rpi5`.

Expected noise: every remote run prints `[xsync server] ...` diagnostic lines.
Those come from the *remote* process's stderr and are drained and echoed
locally. `-q` silences local stdout but **not** these lines. For cron, redirect:

```bash
xs -q ~/Documents/notes/ freya:/srv/backup/notes/ 2>/dev/null
```

---

## 9. Tuning for a home LAN

- **`--streams` defaults to 1.** Multi-stream striping exists and is honored up
  to 16, but the shipping default is deliberately 1 — benchmarks in
  [TUNING.md](TUNING.md) found 2–4 often helps and 8 sometimes regresses badly,
  filesystem-dependent. Measure on your own routes before raising it; the rpi5's
  SD/USB storage and freya's pool will not agree.
- **zstd compression is on by default**, level 3, chosen by a bounded 64 KiB
  sample: it compresses only when the sample comes out at ≤ 95% of the input.
  Already-compressed corpora (the `.cbz` files under `corpora/Manga/`, video,
  photos) will correctly skip compression on their own. `--no-compress` forces
  it off; `--compress-level L` overrides the level.
- **Local same-filesystem copies** try a directory clone / reflink first
  (`clonefile` on macOS, `cp --reflink=always` on Linux). On the Mac's APFS this
  is dramatically faster than a byte copy. On Linux it only helps on btrfs or
  XFS with reflink enabled — ext4 and ZFS fall back to a normal copy.
- **`--checksum` is worth it on repeat syncs** of a large stable tree, because of
  the hash cache. It is a waste on a first sync.

---

## 10. Known issues, verified

### 10.1 The working tree's integration suite is red — stale harness

8 of 22 tests in `crates/xsync/tests/server_integration.rs` fail on the working
tree and pass at `HEAD`.

Cause: the uncommitted change to `xsync_remote_command`
([server.rs:5857](crates/xsync-core/src/server.rs:5857)) prefixes the remote
command with `PATH="$HOME/.local/bin:$PATH" `. The test helper `write_fake_rsh`
([server_integration.rs:92](crates/xsync/tests/server_integration.rs:92)) does
`eval "set -- $1"` and then `exec {target} "$2" "$3"`, which assumed
`$1`=`xs`, `$2`=`--server`, `$3`=`<path>`. The new prefix shifts every
positional by one, so the fake server is launched as `xs xs --server` — rooted at
a relative directory literally named `xs`. Hence the nonsense manifests and the
`invalid chunk range for 'big.bin': offset 0, length 67108864, file size
20971520` error.

The unit test inside `server.rs` *was* updated for the new string; the
integration harness was not.

Fix: shift the indices to `"$3"` and `"$4"`, or drop the positional juggling
entirely and use the `exec /bin/sh -c "$*"` form that
`write_fake_rsync_rsh` ([server_integration.rs:147](crates/xsync/tests/server_integration.rs:147))
already uses. The engine needs no change.

### 10.2 `--delete` under-reports on remote transfers

Verified on the working tree:

| Transfer | Extraneous file removed? | Summary line |
|---|---|---|
| local → local | yes | `1 deleted` ✓ |
| local → remote | **yes** | `0 deleted` ✗ |

The deletion happens correctly; only the counter is wrong on the remote path.
`--dry-run --delete` *does* list `delete <path>` correctly, so the preview is
trustworthy even though the summary is not.

**Operational rule:** preview every `--delete` with `-n` first, and never use the
summary counter as confirmation that a remote mirror pruned what you expected.

### 10.3 The rsync fallback is narrower than it looks

`--transport=auto` falls back to rsync only when the remote `xs` is missing, and
that fallback:

- is **push only** — remote → local always requires native `xs` on the far end;
- requires **GNU rsync advertising protocol 32** (3.4.x/3.5.x). macOS's
  `/usr/bin/rsync` is openrsync protocol 29 and is **rejected**:
  `unsupported remote rsync: openrsync 2.6.9 compatible advertises protocol 29;
  v1 requires GNU rsync protocol 32`;
- rejects `--streams > 1`, `--delete`, `--paranoid`, `--checksum`, and
  `--compress-level` (compression is simply not offered on this path).

Your Linux hosts ship GNU rsync, so the fallback is real there. Do not count on
it for the Mac.

### 10.4 Version strings cannot distinguish builds

`xs --version` reports `xs 0.1.0` on every host, even though the most recent
commit is titled `release: prepare v0.1.1` — the workspace version in
`Cargo.toml` was never bumped. Peer compatibility is checked on
`PROTOCOL_VERSION` (currently `1`), not the semver, so mixed builds interoperate
as long as the protocol version matches.

Protocol v2 work is in flight and uncommitted. **Until it lands, stage every host
from the same commit at the same time**, and re-stage all four together whenever
you rebuild.

### 10.5 Sparse files

A sparse file is read and written at its *logical* size. Per
[DEPLOYMENT.md](DEPLOYMENT.md) Story D8.3, a 130 GB sparse image currently reads
and writes 3.7 TB and cannot complete. Exclude VM images, thin-provisioned disks,
and `.img` files until TUNING-TASKS Epic T2 lands.

### 10.6 No resume across a whole-run failure for small files

Durable chunk-level resume covers large files. Small and medium files are
whole-file work items; an interrupted run re-sends them. Staging is atomic, so a
failure never leaves a truncated file at the final pathname.

---

## 11. State on disk, and how to clean it up

Nothing removes these automatically.

| What | Where |
|---|---|
| BLAKE3 hash cache | `$XDG_CACHE_HOME/xsync/hashes.redb`, else `~/.cache/xsync/hashes.redb` |
| Resume journals | `$TMPDIR/xsync-resume-<16-hex>` |
| Staging files | `.xsync.tmp.<hash>` beside the destination file |

A `.xsync.tmp.*` file left behind is the signature of an interrupted transfer.
Re-running the same command is the correct response — it is safe, and the resume
journal will skip verified ranges of a large file.

---

## 12. What "seamless" still needs

Ordered by what actually blocks daily use on this network.

1. **Fix the integration harness** (§10.1). One line. Do this before anything
   else — it is currently masking any real regression in the v2 work.
2. **Fix the `--delete` counter on the remote path** (§10.2). The behavior is
   right and the reporting is wrong, which is the dangerous combination.
3. **Commit the `PATH="$HOME/.local/bin"` prefix**, or decide against it. Right
   now the remote-PATH contract differs between `HEAD` and the working tree,
   which makes §5.1 unanswerable in general.
4. **Make Windows work as a server** (§7): fix `xsync-server.cmd`'s `xsync.exe`
   → `xs.exe`, teach `is_missing_xsync_stderr` the cmd.exe wording, and decide
   whether to detect a Windows peer and emit a cmd-compatible command line.
5. **A release artifact** (DEPLOYMENT.md D2.2). CI already builds all Tier 1
   targets; it just never uploads them. A tarball per target plus `SHA256SUMS`
   would replace §4.3 entirely and, importantly, give mars-Windows a binary
   without a Visual Studio install.
6. **Bump the version and stamp the commit** (D2.1, §10.4). Four hosts running
   binaries that all claim `0.1.0` is a debugging trap waiting to happen.
7. **A scheduled sync.** Nothing here is a daemon; every transfer is a one-shot
   command. A systemd timer per Linux host and a launchd agent on the Mac
   (D6.1/D6.2) is the smallest thing that turns this from "a tool I run" into
   "a network that stays in sync."
8. **Remote → remote**, or a documented pattern for it. §3's `ssh freya 'xs ...'`
   works and is honest, but it is a workaround, and it fails the moment the Mac
   is the only host with credentials for both ends.
