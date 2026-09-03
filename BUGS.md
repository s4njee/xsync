# Bug review and remediation

Review date: 2026-08-29

The review covered the CLI job/config path, local planning and deletion, the
scanner/filter layer, sink publication, path-semantics probes, clone fast paths,
and native remote sessions.

## Fixed in this change

- **Temporary-file symlink writes:** regular-file staging now uses
  `O_NOFOLLOW` on Unix and rejects symlink staging paths before fallback cleanup.
  This prevents a pre-existing predictable staging symlink from redirecting a
  write outside the destination.
- **Destructive path probes:** case and Unicode probes now use unique temporary
  prefixes and create-new files. They no longer remove user-owned fixed names.
- **Dry-run mutation:** local dry runs do not create a missing destination root;
  a missing destination is treated as an empty scan for planning.
- **Nested ignore parse failures:** errors loading malformed or unreadable
  nested `.xsyncignore` files are retained by the scanner and returned from
  `Scan::finish` instead of causing a fail-open walk.
- **Destination ignore contamination:** destination planning reuses source
  ignore rules without discovering destination ignore files, preventing stale
  destination rules from changing `--delete` decisions.
- **Native push filtering:** local-to-remote native pushes now scan the source
  with the complete ordered filter, including per-directory `.xsyncignore`.
- **Named-job excludes:** config-derived job excludes are included in the
  filter even though clap has no argument indices for them; command-line rules
  retain precedence.
- **Named-job path ambiguity:** `--job` now rejects explicit positional `SRC`
  or `DEST` arguments instead of silently replacing them.
- **Named-job failure logging:** jobs are resolved before failure logging is
  configured, so a job-level `log_json` setting is active for transfer setup.
- **Clone publication:** regular files and symlinks are atomically renamed over
  existing destinations; removal is now limited to the directory-replacement
  case.
- **Large push publication ordering:** when `--paranoid` is enabled, the staged
  large file is now read and checked before `finish_large` publishes it.
- **Remote segment integrity:** `FileSegment` now carries a BLAKE3 digest of
  its payload, and the native sync wire version is bumped to v2 so receivers
  compare against a sender-provided value.
- **Multi-stream source buffering:** data sessions now read only their assigned
  bounded ranges from a stable source descriptor instead of buffering a whole
  large file once per stream.
- **Path-collision skip:** skip policy now drops descendants of colliding
  directory paths instead of merging the child trees into one destination.

## Still open and requiring protocol or larger design work

### P0 — Complete-digest verification for large remote pulls

The remote pull path reads the committed large file under `--paranoid` and
discards the bytes, then sends an all-zero `LargeFileFinish` digest. Large-file
pulls still need a sender-provided complete digest and a comparison before
publication; per-range digests now protect each received segment, but cannot
prove the complete file's ordering and contents alone.

### P1 — Windows publication is not crash-atomic — **fixed 2026-09-03** (`xsyncv3.md` E4-S6)

`std::fs::rename` on Windows is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`,
which already replaces an existing file atomically. The unconditional
`remove_existing` was the whole bug; deleting it is the whole fix. A destination
that is a *directory* is still handled, on the failure path, the way the Unix
implementation handles it.

Original report:

The Windows `commit_temp` implementation removes the existing destination
before renaming the staged file. A crash or rename failure can leave no
destination. Use a Windows replace/rename primitive with replacement semantics,
or retain and restore the old path on failure.

## Verification

The focused regression tests for staging symlinks and malformed nested ignore
files pass. The workspace test suite passes with **325 tests passed and 2
ignored** after these changes. `cargo check` and `cargo fmt --all -- --check`
also pass.
