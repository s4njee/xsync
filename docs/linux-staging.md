# Linux Binary Staging

`13.2` stages Linux GNU binaries from the macOS development machine. The
supported targets are the Tier 1 targets from `docs/TARGET-MATRIX.md`:

| Client argument | Target | Benchmark hosts |
|---|---|---|
| `amd64` | `x86_64-unknown-linux-gnu` | x86_64 ZFS, x86_64 ext2/3 |
| `arm64` | `aarch64-unknown-linux-gnu` | aarch64 ext4 |

## Prerequisites

Install `zig` and `cargo-zigbuild` on macOS. The remote host needs only SSH,
`sha256sum` or `shasum`, `mv`, `rm`, `mkdir`, and an executable filesystem.
It does not need Rust, Cargo, a package manager, root, or a compiler.

## Usage

```text
scripts/stage-linux.sh host /home/user/bin/xs amd64
scripts/stage-linux.sh host /home/user/bin/xs arm64
scripts/deploy-mars.sh
```

`deploy-mars.sh` defaults to `mars.local`, resolves the remote user's home, and
stages the latest checkout at `~/.local/bin/xs`. Override the destination or
architecture with `scripts/deploy-mars.sh [host] [remote-path] [amd64|arm64]`.

The binary is uploaded beside the destination under a temporary name. The
script verifies its SHA-256 before changing the destination, then uses `mv` to
publish it. An interrupted upload therefore leaves the old binary untouched;
the temporary file is cleaned up on exit. Repeating the command is safe and
replaces the destination only after verification.

The build command is `cargo zigbuild --release --target <triple> -p xsync`.

## Verified Run

On 2026-08-25 from macOS, against `192.168.1.119` (`mars`, x86_64/ext4):

| Target | Build time | Artifact size | Result |
|---|---:|---:|---|
| `x86_64-unknown-linux-gnu` | 53.97 s | 4,905,688 bytes | staged and ran `xs 0.1.0` |
| `aarch64-unknown-linux-gnu` | 54.24 s | 4,139,344 bytes | staged and verified as ARM ELF |

The remaining ZFS, ext2/3, and aarch64 ext4 hosts require their own staging
checks before the story is complete.
