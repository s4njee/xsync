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

Audited 2026-08-26 across 115 dependencies with `--all-features`. Every licence
in the tree is permissive and compatible with the declared `MIT OR Apache-2.0`.

| Count | Licence expression |
|------:|---|
| 82 | `MIT OR Apache-2.0` |
| 9 | `MIT` |
| 6 | `MIT/Apache-2.0` |
| 5 | `Unlicense OR MIT` |
| 5 | `Apache-2.0 OR MIT` |
| 2 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| 2 | `Unlicense/MIT` |
| 1 | `CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception` |
| 1 | `CC0-1.0 OR MIT-0 OR Apache-2.0` |
| 1 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` |
| 1 | `(MIT OR Apache-2.0) AND Unicode-3.0` |

There is no copyleft-only dependency. The single crate offering
`LGPL-2.1-or-later` offers it as one option among three; the build takes its MIT
option, so no LGPL obligation attaches.

## Accepted findings

| ID | Crate | Why accepted |
|---|---|---|
| `RUSTSEC-2025-0119` | `number_prefix` 0.4.0 | Unmaintained, not vulnerable. Transitive via `indicatif`, used only to format human-readable byte counts for progress output. Parses no untrusted input, contains no unsafe code. Revisit when `indicatif` drops it. |

## Duplicate versions

Three crates resolve to two versions each: `shlex`, `syn`, and `windows-sys`.
All are forced by upstream dependencies rather than by this workspace's own
manifests, so `deny.toml` sets `multiple-versions = "warn"`: new duplicates stay
visible without a transitive bump breaking the build.
