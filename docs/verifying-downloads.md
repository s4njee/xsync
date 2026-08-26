# Verifying a Download

Story D3.3. Every release publishes `SHA256SUMS` and a build provenance
attestation. This is how to check an artifact is what it claims to be.

## The short version

```bash
gh attestation verify xsync-0.1.0-aarch64-apple-darwin.tar.gz --repo <owner>/xsync
```

That is the strongest check available and needs no key management. It verifies,
via Sigstore, that this exact file was produced by this repository's release
workflow from a specific commit — not merely that it matches a checksum someone
published next to it.

## Without the gh CLI

```bash
sha256sum --check --ignore-missing SHA256SUMS      # Linux
shasum -a 256 -c SHA256SUMS --ignore-missing       # macOS
```

Be clear about what this proves: only that the file matches the list. If an
attacker can replace the artifact they can replace `SHA256SUMS` beside it, so a
checksum alone protects against corruption in transit, not substitution. Use the
attestation when you need the stronger property.

## Why not a GPG key

Detached GPG signatures move the problem rather than solving it: the user has to
obtain the right public key through some *other* trusted channel, and in
practice almost nobody does. The provenance attestation binds the artifact to
the workflow run and the commit that produced it, with no key for anyone to
distribute, verify, or rotate — and no private key for this project to keep
secret and eventually lose.

If a signing key is introduced later it needs a documented rotation policy and a
publication channel independent of the release page, or it adds ceremony without
adding trust.

## Platform trust: what to expect

**macOS.** Binaries are ad-hoc signed by the linker, which is what lets them run
on Apple Silicon at all, but they are **not** signed with a Developer ID
certificate and **not** notarized.

Gatekeeper only blocks files carrying the `com.apple.quarantine` attribute, and
that attribute is applied by the *downloading* application — browsers, Mail,
AirDrop. It is not applied by `curl`, `wget`, `git`, Homebrew, or `cargo`.

So:

| How you obtained it | Result |
|---|---|
| `curl`/`wget`, Homebrew, `cargo install` | Runs normally |
| Downloaded in a browser and unzipped in Finder | Gatekeeper refuses it |

In the second case, the file can be cleared with:

```bash
xattr -d com.apple.quarantine xsync-0.1.0-aarch64-apple-darwin.tar.gz
```

Telling a user to strip a security attribute is not a substitute for signing. It
is stated here so the situation is understood rather than discovered, and it is
the reason the install instructions use `curl` rather than a download link.

**Windows.** The `.exe` is not Authenticode-signed. SmartScreen weighs
reputation, which unsigned binaries do not accumulate, so a browser download may
show "Windows protected your PC" with a **More info → Run anyway** step.
Fetching through `curl`, `winget`, or `scoop` does not go through that path.

An ordinary (OV) certificate does not fix this immediately — reputation accrues
per-certificate over time and download volume. Only an EV certificate carries
immediate SmartScreen reputation. This is worth knowing before buying one and
finding the warning still appears.

## If signing is adopted later

`.github/workflows/release.yml` is written so signing steps activate only when
their secrets are present, and **fail closed** rather than silently publishing
unsigned artifacts when a secret is configured but unusable. Adding certificates
is therefore a secrets change, not a pipeline rewrite.
