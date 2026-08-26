# Deferred Questions

Design questions deliberately postponed, with enough context to pick them up cold.
Each entry states the current behaviour, why it is questionable, and what a fix
would cost.

---

## Non-UTF-8 filenames are silently mangled on Windows

**Raised:** 2026-08-26, during first native Windows (ARM64) bring-up.
**Status:** open — current behaviour left as-is.

`WirePath::to_native_path` (`crates/xsync-core/src/path.rs`) converts wire bytes
to a native path. The Windows branch is:

```rust
return root.join(String::from_utf8_lossy(&self.0).as_ref());
```

`from_utf8_lossy` replaces every invalid sequence with `U+FFFD` and reports
nothing. `docs/TARGET-MATRIX.md` already states that filenames which cannot be
represented as UTF-8 are not supported on Windows, so this is within the
documented contract.

**Why it is still worth revisiting.** For a synchronisation tool, "unsupported"
is being implemented as "write to a different path than the one requested"
rather than "refuse to act". Two consequences:

- Two source files differing only in their invalid byte sequences collapse onto
  a single destination path. One silently overwrites the other, and the transfer
  reports success.
- `--delete` compares a mangled destination name against the real source name.
  A file that was written under a substituted name is a deletion candidate on
  the next run.

Neither is detectable from the destination afterwards: `U+FFFD` is a legitimate
character, so a mangled name is indistinguishable from a real one.

**What a fix costs.** `to_native_path` currently returns `PathBuf` infallibly.
Rejecting non-UTF-8 on Windows means returning `Result`, which touches every
caller — the same call sites listed below, plus the scanner and sink paths. That
is why it was not bundled with the compile fix.

**Related work already done.** The `#[cfg(unix)]`-gated `AsRef<Path>` /
`AsRef<OsStr>` impls on `WirePath` meant `root.join(wire_path)` compiled on Unix
and failed on Windows. Seven such sites were rewritten to call `to_native_path`
(`local.rs` 644, 1355, 1474, 1532; `server.rs` 2964, 3448, 5341). That fix made
Windows compile but deliberately did **not** change the lossy semantics.

**Decision needed:** reject non-UTF-8 wire paths on Windows with a typed error,
or keep lossy conversion and document the overwrite/delete hazard explicitly.

---

## `--modify-window` for coarse-granularity filesystems

**Raised:** 2026-08-26, alongside the mtime-granularity fix below.
**Status:** open — not implemented.

Modification times are now compared at whole-second granularity (see the entry
below), which covers NTFS (100 ns ticks), HFS+ and ext3 (1 s). It does **not**
cover FAT/exFAT, which quantise to 2 seconds — a common case for USB backup
targets. rsync solves this with `--modify-window=NUM`.

Implementing it means threading a `modify_window: Duration` from the CLI through
`LocalSyncOptions` into `planner::metadata_matches`. Planning is sender-side
only, so no protocol change is required. It was left out of the granularity fix
to keep that change to one behavioural decision.

---

## RESOLVED 2026-08-26: unchanged files were never skipped across filesystems

Kept for the reasoning; the fix is in `planner::mtimes_match`.

`metadata_matches` compared `source.mtime == destination.mtime` — exact
`SystemTime` equality at nanosecond precision. Filesystems quantise
modification times differently, so a timestamp does not survive a round trip
between them:

| Direction | 2nd sync | mtime |
|---|---|---|
| Windows -> Windows (NTFS -> NTFS) | 2 skipped | `.624189100` == `.624189100` |
| macOS -> Windows (APFS -> NTFS) | 0 skipped, 8 transferred | `.126080149` != `.126080100` |

APFS and ext4 store nanoseconds; NTFS stores 100-nanosecond ticks. The
sub-100 ns remainder is truncated on write, so any file whose nanosecond
component is not a multiple of 100 was classified as modified on **every**
subsequent run. Every macOS-to-Windows sync was a full re-transfer, which
defeats the purpose of an incremental sync tool. Linux ext4 to Windows was
affected identically.

Fixed by comparing whole seconds, which is what rsync does.

**The cost of that choice, measured.** Second granularity cannot distinguish two
writes inside the same second when the size is unchanged. Verified on macOS
local-to-local: write `AAAA`, sync, overwrite with `BBBB` in the same second,
sync again — the second run reports `1 skipped` and the destination still holds
`AAAA`. `--checksum` catches it (`1 transferred`, destination `BBBB`), which is
why fixing the `--checksum` semantics was a prerequisite rather than a separate
nicety.

Note this is a real reduction in sensitivity for same-filesystem syncs, which
previously compared nanoseconds and would have caught that edit. It is the same
trade rsync makes, and it buys correct behaviour for every cross-filesystem
pair. If the loss matters more than the simplicity, the alternative is to accept
the destination timestamp when it is exactly a truncation of the source
timestamp at some filesystem granularity (100 ns, 1 us, 1 ms, 1 s), which keeps
nanosecond sensitivity when both sides support it. That is more code and a less
predictable rule, so it was not chosen without a reason to.

---

## RESOLVED 2026-08-26: `--checksum` augmented size+mtime instead of replacing it

Kept because the CLI wording is the contract and it had drifted from the code.

`crates/xsync/src/main.rs` documents `--checksum` as "Classify by content hash
(BLAKE3) instead of size+mtime". `metadata_matches` required the mtime
comparison unconditionally and only *added* the content-hash check:

```rust
source.kind == destination.kind
    && source.size == destination.size
    && source.mtime == destination.mtime          // always required
    && (!compare_fingerprint || ... identity == ...)   // checksum only ADDED
```

Two consequences. The flag did not do what it documented, and — because the
mtime comparison still ran — it could not serve as the workaround for the
granularity bug above. A user hitting full re-transfers on Windows would reach
for `--checksum`, observe no improvement, and have nothing left to try.

Note that `fingerprint.identity` is overloaded: normally an inode/device pair
for same-file detection, but overwritten with a BLAKE3-derived value by
`cached_content_identity` when `--checksum` is set. Anything touching that field
needs to know which meaning is in play.

---

## Remote-shell family is cached per process, not persisted

**Raised:** 2026-08-26, with the Windows remote-shell support.
**Status:** open — works correctly, costs one extra connection per invocation.

`remote_shell_for` remembers the learned family in a process-lifetime map. A
job with `--streams 8` therefore probes nothing and pays the discovery cost
once. But a *new* `xs` invocation starts with an empty map, so every run against
a Windows host pays one failed POSIX attempt before retrying with the cmd form:
two SSH connections per invocation instead of one.

Persisting the map (`~/.cache/xsync/remote-shells`, keyed the same way) would
reduce that to one connection for every run after the first. Deferred because it
introduces cache invalidation questions that the in-process map does not have —
a host that changes its `DefaultShell`, or a name that resolves to a different
machine, would need the entry re-learned rather than trusted indefinitely.

Note the failed first attempt is harmless: it is the `PATH` builtin setting a
junk search path inside a shell that then exits. Nothing is written and no
process is started on the remote.

---

## The rsync fallback rejects the rsync shipped by current Ubuntu LTS

**Raised:** 2026-08-26, while making the Linux CI job pass.
**Status:** open — product decision, not a defect in the code as written.

`rsync::validate_peer` requires `GNU rsync` advertising protocol >= 32
(`RSYNC_WIRE_VERSION`). The GitHub `ubuntu-latest` runner's system rsync
advertises a lower protocol, so xsync refuses it during the version probe.

This matters more than a skipped test. The rsync transport exists so that a
remote *without* xsync still works — DEPLOYMENT.md D5.1 offers it explicitly as
the fallback when the remote binary is missing, and it is the reason
"works with what is already installed" is claimed at all. If the protocol floor
excludes the rsync on current Ubuntu LTS, that fallback does not fire on a large
share of realistic Linux destinations, and those users get a hard failure rather
than a degraded transfer.

Two ways forward, both requiring a decision rather than a patch:

- Lower the floor to protocol 31 and state what is lost. This needs an audit of
  which wire features the sender relies on above 31, plus test vectors against a
  protocol-31 peer.
- Keep the floor and make the failure actionable: name the peer's protocol, say
  that xsync requires 32, and point at installing xsync on the remote (D5.2
  bootstrap) as the real fix rather than reporting an opaque unsupported-peer
  error.

Until then `test_rsync_nonzero_receiver_exit_is_failure` and its siblings skip
wherever no protocol-32 rsync exists, which is honest but means Linux CI does
not exercise the rsync transport at all. Installing a protocol-32 rsync in the
CI job would restore that coverage independently of the decision above.
