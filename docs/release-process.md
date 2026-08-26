# Release Process

Epic D2. One tag produces every artifact.

## Cutting a release

1. Update `version` in `Cargo.toml` (`[workspace.package]`) — the single source
   of truth.
2. Add a `CHANGELOG.md` section for it, stating the protocol version and whether
   it is wire-compatible with the previous release.
3. Tag `vX.Y.Z` and push. The `Release` workflow does the rest.

## Guards

The pipeline refuses before building anything if:

- the tag and `Cargo.toml` disagree — a binary reporting a version different
  from the tag it shipped under is worse than no version at all;
- `CHANGELOG.md` has no section for the version;
- `LICENSE-MIT` or `LICENSE-APACHE` is missing.

`gh release create --verify-tag` refuses a tag that does not exist on the remote.

## Dry run

`workflow_dispatch` runs `verify` and `build`, uploads the artifacts, and stops
before publishing. Use it to exercise the pipeline without minting a version —
a release process first tested during a release is tested in the worst possible
place.

## Artifacts

Per Tier 1 target (`docs/TARGET-MATRIX.md`):

- `xsync-<version>-<target>.tar.gz`, or `.zip` for Windows
- each containing the binary, `LICENSE-MIT`, `LICENSE-APACHE`, and `README.md`
- `SHA256SUMS` covering all of them
- build provenance attestation via `actions/attest-build-provenance`

`scripts/package-release.sh <target> [outdir]` produces one artifact and is what
CI calls, so the packaging can be run and debugged locally rather than only
inside a workflow.

## Reproducibility

Two builds from the same commit produce byte-identical binaries **and**
byte-identical archives. Verified on `aarch64-apple-darwin`:

```
run 1: 358fe30061f8357fc5e3b8a1ce02f471a78d1479
run 2: 358fe30061f8357fc5e3b8a1ce02f471a78d1479
```

What it takes, since none of it is default:

- **`SOURCE_DATE_EPOCH`** — the build stamps a date, and without pinning it two
  builds of one commit differ on that alone. The script defaults it to the
  commit's own timestamp, so the value is a property of the source rather than
  of when the build ran.
- **Normalised mtimes before archiving.** GNU tar's `--mtime`/`--sort`/`--owner`
  are not accepted by the bsdtar macOS ships. The first version of this script
  passed them and silently fell through to a plain `tar -czf` there, producing
  archives that differed between runs even though the binaries inside were
  identical. Setting times on disk with `touch` works under either tar.
- **`gzip -n`** so the compressor does not stamp its own timestamp.
- **An explicit, sorted, files-only member list.** Naming the staging directory
  as well made tar recurse into it *and* archive each member separately, which
  doubled the artifact while still hashing identically between runs.
  Reproducible and wrong is still wrong.

Not yet verified: reproducibility *across machines* (different toolchain paths
can leak into debug info) and for the cross-compiled targets. The remaining
known source of nondeterminism is the absolute path embedded by rustc, which
`--remap-path-prefix` would address.
