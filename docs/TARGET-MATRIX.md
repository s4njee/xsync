# Supported Target Matrix

This is the release target contract for xsync. Targets not listed here are not
built or supported by the release process.

## Tier 1

Tier 1 targets are built, tested, signed, and released for every version tag.

| Target triple | Platform | ABI baseline |
|---|---|---|
| `aarch64-apple-darwin` | macOS ARM64 | macOS 11 |
| `x86_64-apple-darwin` | macOS x86_64 | macOS 10.15 |
| `x86_64-unknown-linux-gnu` | Linux x86_64 | glibc 2.28 |
| `aarch64-unknown-linux-gnu` | Linux ARM64 | glibc 2.28 |
| `x86_64-pc-windows-msvc` | Windows x86_64 | MSVC |

The Linux glibc floor is 2.28. Linux GNU release binaries must be built in
`build/linux-glibc`, not on an arbitrary newer host, so the link environment
cannot silently raise that floor.

## Tier 2

Tier 2 targets are built and tested when their builders are available. They are
not necessarily signed.

| Target triple | Purpose |
|---|---|
| `aarch64-pc-windows-msvc` | Windows ARM64 |
| `x86_64-unknown-linux-musl` | Static Linux x86_64 bootstrap binary |
| `aarch64-unknown-linux-musl` | Static Linux ARM64 bootstrap binary |

## Not Built

All other target triples, including 32-bit, i686, Windows GNU, and other Unix
targets, are outside the v1 release contract and are not built.

## Build Commands

From the repository root, native builders use:

```text
cargo build --release --target <target-triple>
cargo test --target <target-triple>
cargo clippy --all-targets --target <target-triple> -- -D warnings
```

The musl builder uses the same commands with the musl target and supplies the
musl linker through the image toolchain. Cross-target tests require an executor
or emulator and are therefore a CI concern covered by D1.

## Windows Filesystem Notes

Windows integration tests cover drive-letter paths, backslash paths, long paths,
and case-insensitive destination collisions. Windows filenames are represented
as UTF-8 protocol paths by the current v1 scanner; filenames that cannot be
represented as UTF-8 are therefore not supported on Windows. Unix tests retain
coverage for raw non-UTF-8 filenames.
