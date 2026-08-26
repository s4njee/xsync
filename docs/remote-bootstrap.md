# Remote Bootstrap

Story D5.2. `--bootstrap` places a verified xsync binary on a remote that has
none, so a host reachable over SSH can be synced to without installing anything
on it by hand.

It is **off by default**. Copying an executable to another machine and running
it is the shape of a supply-chain attack, so it never happens implicitly.

```
xs --bootstrap=once    ./data  host:/backup   # upload, use, remove
xs --bootstrap=persist ./data  host:/backup   # upload and leave in place
```

## What happens

1. **Detect.** One probe reports the remote's OS, architecture, C runtime, and
   home directory. `uname` on POSIX, `%OS%`/`%PROCESSOR_ARCHITECTURE%` under
   `cmd.exe`.
2. **Choose a binary.** The running executable is used when its own target
   triple matches the remote. Otherwise a matching binary must exist under
   `$XSYNC_BOOTSTRAP_DIR/<triple>/xs` or `~/.cache/xsync/binaries/<triple>/xs`.
   If none does, xsync **refuses** and names the triple it needs and every path
   it searched — a binary for the wrong architecture or libc fails on the remote
   with an error that explains nothing, so the refusal is the useful outcome.
3. **Upload.** Over `scp`.
4. **Verify.** The remote computes the SHA-256 with its own preinstalled tool
   (`sha256sum`, `shasum`, or `certutil`) and it is compared to the digest of
   the bytes that were sent. **This happens before the binary is executed.** A
   mismatch deletes the file rather than leaving an unverified executable
   behind.
5. **Run**, then remove it if the policy was `once`.

## Guarantees

- Nothing is installed system-wide and nothing needs root. Uploads go only under
  the invoking user's own home directory.
- The binary is checksummed on the remote before it is executed.
- `once` leaves nothing behind.
- Bootstrap is reported on stderr even under `--quiet`: uploading an executable
  to another machine is a side effect the operator should always see.

## Where the binary lands

| Policy | Path | Why |
|---|---|---|
| `once` | `<home>/.cache/xsync/xs-<digest>` | Digest-tagged, so repeat and concurrent runs converge on one file instead of racing over a name. Removed afterwards. |
| `persist` | `<home>/.local/bin/xs` | The directory the remote command already prepends to `PATH`, so later runs — including runs from a different machine, which have no memory of this one — find it as a plain `xs`. A digest-tagged path would survive the run while remaining undiscoverable. |

## Why scp rather than a shell pipe

Binary stdin to a Windows remote is not dependable. Measured against
OpenSSH-for-Windows: 50 KB and 200 KB piped to the login shell arrived intact, a
1 MB payload arrived truncated at varying lengths. `scp` speaks the sftp
protocol and transferred 3 MB byte-identical.

The consequence is that bootstrapping a **Windows** remote requires the default
`ssh` transport, because an `scp` command cannot be derived from an arbitrary
`-e/--rsh`. That combination is refused with a message saying so rather than
transferring a truncated executable. POSIX remotes are unaffected.

## Limitations

- There is no release channel to fetch a binary from yet (D2.2), so
  cross-platform bootstrap needs one staged locally. Same-platform bootstrap
  works with no setup, because the running executable is used.
- 32-bit and non-Tier targets are refused; see `docs/TARGET-MATRIX.md`.
