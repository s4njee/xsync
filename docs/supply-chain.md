# Supply Chain

Story D1.3. Policy lives in [`deny.toml`](../deny.toml) and is enforced in CI by
the `supply-chain` job. This file records the audit result so a change in the
dependency tree is visible as a diff rather than discovered during a release.

## Guarantees

- `Cargo.lock` is committed and every CI job builds with `--locked`, so CI
  resolves exactly the versions recorded here.
- The workspace denies `unsafe_code` (`[workspace.lints.rust]` in `Cargo.toml`).
- `cargo audit` and `cargo deny check` run on every push and pull request, and
  fail on any finding not explicitly accepted in `deny.toml`.
- All dependencies come from crates.io; unknown registries and git sources are
  denied outright.

## Licence audit

Re-audited 2026-09-03 across 135 packages with `--all-features` (the four
workspace members included). Every licence in the tree is permissive and
compatible with the declared `MIT OR Apache-2.0`. The previous audit, on
2026-08-26, saw 115.

| Count | Licence expression |
|------:|---|
| 96 | `MIT OR Apache-2.0` |
| 11 | `MIT` |
| 7 | `Apache-2.0 OR MIT` |
| 6 | `MIT/Apache-2.0` |
| 5 | `Unlicense OR MIT` |
| 2 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| 2 | `Unlicense/MIT` |
| 1 | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| 1 | `CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception` |
| 1 | `CC0-1.0 OR MIT-0 OR Apache-2.0` |
| 1 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` |
| 1 | `MIT OR Apache-2.0 OR Zlib` |
| 1 | `Zlib OR Apache-2.0 OR MIT` |

`Zlib` appears as one option in two dual/triple licences; the build takes an
allowed option in both, so it is not added to `deny.toml`'s allow list.

There is no copyleft-only dependency. The single crate offering
`LGPL-2.1-or-later` offers it as one option among three; the build takes its MIT
option, so no LGPL obligation attaches.

## Direct dependencies added since the previous audit

| Crate | Added by | Why, and what was considered |
|---|---|---|
| `fs4` 1.1.0 | `xsyncv3.md` E5-S3 (`StatFs`) | Free, available and total space for the mounted export. The workspace denies `unsafe_code`, so `statvfs` and `GetDiskFreeSpaceEx` are unreachable directly, and capacity is a stated requirement of the client this serves. `fs4` is the maintained successor to `fs2`, is `MIT OR Apache-2.0`, and its default feature set is `sync` only — no async runtime enters the tree. It does not expose inode counts or the filesystem type name, which is why `FsInfo` reports those as unknown rather than guessing. |

## Accepted findings

| ID | Crate | Why accepted |
|---|---|---|
| `RUSTSEC-2025-0119` | `number_prefix` 0.4.0 | Unmaintained, not vulnerable. Transitive via `indicatif`, used only to format human-readable byte counts for progress output. Parses no untrusted input, contains no unsafe code. Revisit when `indicatif` drops it. |

## Duplicate versions

Three crates resolve to two versions each: `shlex`, `syn`, and `windows-sys`.
All are forced by upstream dependencies rather than by this workspace's own
manifests, so `deny.toml` sets `multiple-versions = "warn"`: new duplicates stay
visible without a transitive bump breaking the build.
