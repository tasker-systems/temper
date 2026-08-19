# Spike: sigstore crate selection for pinned-root attestation verification

**Date:** 2026-07-29
**Plan task:** Task 1 of `internal/superpowers/plans/2026-07-29-binary-attestation-and-manifest-verification.md`
**Spec:** `internal/superpowers/specs/2026-07-29-binary-attestation-and-manifest-verification-design.md`

## The question

> Can `sigstore-verification` or `sigstore-verify` verify a real GitHub attestation bundle against a
> **caller-supplied, pinned** trust root, with no network TUF fetch?

## Decision

**`CRATE = sigstore-verify@0.11.0`** (with `sigstore-trust-root@0.11.0` and `sigstore-types@0.11.0`).

**This inverts the pre-spike expectation.** `sigstore-verification` was the leading candidate on the
strength of its crates.io summary ("Sigstore, Cosign, and SLSA attestation verification library") and
its `verify_github_attestation()` entry point. Reading the actual source disqualified it on both
criteria. The purpose-built-sounding API is the wrong shape; the lower-level crate is the right one.

## Evidence

### Fixtures used (real, not fabricated)

```
$ gh attestation trusted-root > trusted_root.jsonl
34K, 2 lines, mediaType application/vnd.dev.sigstore.trustedroot+json;version=0.1

$ gh release download --repo cli/cli --pattern '*checksums.txt'
$ gh attestation download gh_2.96.0_checksums.txt --repo cli/cli
Wrote attestations to file sha256:fc046371…3565.jsonl   (6.2K)
```

`gh attestation trusted-root` is itself a finding: the trust root is materializable as a standard
`TrustedRoot` JSON document. That is the concrete artifact the spec's "pin the trust root" decision
would compile in.

### `sigstore-verification@0.2.8` — REJECTED on both criteria

**1. No caller-supplied trust root. Verification always performs a network TUF fetch.**

```
src/verify.rs:311:  async fn get_sigstore_trust_root() -> Option<Arc<SigstoreTrustRoot>>
src/verify.rs:327:  /// Fetch the Sigstore trust root from the TUF repository
src/verify.rs:328:  async fn fetch_sigstore_trust_root() -> Result<SigstoreTrustRoot>
```

Both functions are **private** and take **no parameter**. The entire public surface —
`verify_attestations`, and `AttestationClientBuilder` whose only builder methods are `base_url` and
`github_token` (`src/api.rs:22,27`) — offers no way to inject a root. The TUF dependency the spec set
out to avoid is not optional here; it is the only path.

**2. Dependency hygiene fails the Global Constraint.**

```
$ cargo tree -e normal | wc -l
905
native-tls v0.2.18        ← violates the reqwest rustls-tls/default-features=false pin
aws-lc-fips-sys v0.13.16  ← FIPS C crypto build
oci-client v0.15.0
reqwest v0.12.28 AND v0.13.4   ← two majors in one tree
```

`native-tls` alone disqualifies it: `temper-cli` pins `reqwest` to `rustls-tls` with
`default-features = false`, and `native-tls` means OpenSSL on Linux.

### `sigstore-verify@0.11.0` — SELECTED

**1. The trust root is a required constructor argument.**

```rust
// src/verify.rs:181
pub fn new(trusted_root: &TrustedRoot) -> Self

// src/verify.rs:570 — the free function, synchronous
pub fn verify<'a>(
    artifact: impl Into<Artifact<'a>>,
    bundle: &Bundle,
    policy: &VerificationPolicy,
    trusted_root: &TrustedRoot,
) -> Result<VerificationResult>
```

**2. The root is loadable from a string — i.e. `include_str!`-able.**

```rust
// sigstore-trust-root-0.11.0/src/trusted_root.rs
pub fn from_json(json: &str) -> Result<Self>       // :235
pub fn from_file(path: impl AsRef<Path>) -> Result<Self>   // :240
pub fn from_embedded(instance: SigstoreInstance) -> Result<Self>  // :596
```

The crate's own doc example does exactly the pin we want:
`TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT)`. This is the `EXPECTED_MODEL_SHA256`
doctrine, available as a first-class API rather than something we bolt on.

**3. No TUF fetch.** The only `tuf` match in the whole source tree is a comment
(`verify_impl/helpers.rs:401`, about a smoke test). The crate verifies; it does not fetch.

**4. Synchronous.** `pub fn verify`, not `async fn` — no runtime needed at the call site.

**5. The identity policy we need is built in.**

```rust
VerificationPolicy::with_identity(..) / require_issuer(..)   // src/verify.rs:69,91
```

That is the cert-identity check the spec requires (issuer = GitHub Actions OIDC, SAN = the workflow
ref at the release tag).

**6. Dependency hygiene passes.**

```
$ cargo tree -e normal | wc -l
442
native-tls:  0
openssl:     0
oci-client:  0
aws-lc-rs v1.17.3   ← rustls' default crypto provider, NOT aws-lc-fips-sys
```

### Executed verification — what actually ran

A throwaway crate outside the workspace loaded the real trusted root and the real bundle and called
`verify`:

```
bundle parsed OK (no network)
root[0]: parsed OK
root[0]: verify failed: bundle validation failed: v0.3 bundle must have inclusion proof
root[1]: parsed OK
root[1]: verify failed: bundle validation failed: v0.3 bundle must have inclusion proof
```

**What this proves:** both lines of the real `gh attestation trusted-root` output parse into
`TrustedRoot`; the real bundle parses; verification runs with a **caller-supplied** root and reaches
bundle-validation logic **with no network access**. The decisive question is answered YES.

**What this does NOT prove — carried as a residual risk, not papered over.** Full
signature verification was **not** demonstrated end-to-end. The run stopped at bundle validation
because the bundle obtained via `gh attestation download` carries no Rekor **inclusion proof**, which
this crate requires for a v0.3 bundle. This is a property of how that fixture was fetched from the
GitHub attestations API, not evidence about trust-root injection.

## RESOLVED (same day): the residual risk is closed

The first fixture was the wrong *kind* of attestation. Re-run against a real SLSA
**build-provenance** bundle — the kind `attest-build-provenance` actually emits, and therefore the
kind our pipeline will produce:

```
$ gh attestation download gh_2.96.0_macOS_arm64.zip --repo cli/cli \
    --predicate-type https://slsa.dev/provenance/v1
$ jq -r '.verificationMaterial|keys[]'
certificate
timestampVerificationData
tlogEntries          ← present
$ jq '.verificationMaterial.tlogEntries[0].inclusionProof'
PRESENT
```

and verification then **succeeds end-to-end, offline, against the pinned root**:

```
bundle parsed OK (no network)
root[0]: VERIFIED (offline, caller-supplied pinned root)
root[1]: failed: certificate chain validation failed: UnknownIssuer
```

**Why the first fixture failed, precisely.** `gh attestation download` without a predicate filter
returned cli/cli's `https://in-toto.io/attestation/release/v0.2` *release* attestation, which is
**TSA-timestamped with no `tlogEntries` at all** — GitHub's own trust domain, not the public-good
Sigstore instance. `sigstore-bundle`'s `validate_v0_3` requires an inclusion proof, so it was
correctly rejected. Nothing was wrong with the crate or the trust root; the fixture was simply not a
build-provenance bundle.

**The negative control matters as much as the pass.** `root[1]` — the second trust root in the same
file — fails with `UnknownIssuer`. A verifier that accepted *any* root would be worthless, so the
fact that the wrong root is rejected is what makes the `VERIFIED` on `root[0]` mean something.

**Consequences for Task 9:**

- No `skip_tlog()`. `VerificationPolicy::default()` works as-is; the transparency-log guarantee is
  kept in full. The `skip_tlog` escape hatch below is **not needed and must not be used**.
- **Pin the public-good root specifically.** The trusted-root file carries more than one root, and
  they are not interchangeable — verification must be attempted against the one that actually signed
  our bundles. Pinning the wrong one fails closed (`UnknownIssuer`), which is the safe direction, but
  it would be a confusing outage.
- Task 9 must fetch the attestation with the SLSA provenance predicate explicitly, not whatever the
  API returns by default.

## Original residual risk (superseded by the section above)

**Task 9 must establish how to obtain a bundle carrying its inclusion proof**, and must not assume
the raw GitHub attestations API response is sufficient. Options to evaluate at implementation time:
fetch the inclusion proof from Rekor and assemble a complete bundle; use the bundle form
`attest-build-provenance` uploads as a release asset if it is complete; or use
`VerificationPolicy::skip_tlog()` (`src/verify.rs:97`) — **the last only with an explicit written
rationale**, since skipping the transparency log removes a real part of the guarantee and would need
to be declared in the docs the way every other limitation in this design is.

This does not change the crate decision: the trust-root question, which is what selects the
architecture, is settled.

## Notes

- `gh attestation verify` against the same fixture also failed offline (`verifying with issuer
  "GitHub, Inc."`), consistent with the fixture being a `https://in-toto.io/attestation/release/v0.2`
  release attestation rather than the `https://slsa.dev/provenance/v1` build provenance our pipeline
  will emit. Our own artifacts will carry the SLSA predicate.
- `aws-lc-rs` is a C build. If it proves troublesome on any release target, rustls can be pointed at
  `ring` instead via feature selection; not required today.
