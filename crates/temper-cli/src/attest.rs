//! Offline verification of a GitHub build-provenance attestation against a
//! PINNED Sigstore trust root — no TUF fetch, no network call, no dependency
//! on any server being reachable at verify time.
//!
//! # Why pinned, and why THIS root specifically
//!
//! This is the `EXPECTED_MODEL_SHA256` doctrine (`crates/temper-ingest/build.rs`)
//! applied one level up: rather than fetching a root of trust at the moment we
//! need it — which is exactly the TUF round-trip this design exists to avoid —
//! the root is committed to the repo and baked into the binary at compile time
//! via [`include_str!`]. A binary that cannot state which root it trusts
//! cannot verify anything against it, so there is no fallback path: a broken
//! embedded root is a hard failure ([`AttestError::TrustRootUnusable`]), never
//! a silent skip.
//!
//! `gh attestation trusted-root` emits **two** roots for a GitHub repo: the
//! public-good Sigstore instance (`rekor.sigstore.dev` / `fulcio.sigstore.dev`)
//! and GitHub's own domain (`fulcio.githubapp.com`, which carries no
//! transparency logs at all). Only the former signs `attest-build-provenance`
//! bundles for public repos — the latter fails our bundles closed with
//! `UnknownIssuer` (see the spike below). We pin **only** the public-good
//! root, at `trust/sigstore-public-good-trusted-root.json`. Pinning the wrong
//! root fails safe, not silently — but it would still be a confusing outage,
//! so this is deliberate, not incidental.
//!
//! # Crate choice (recap; full reasoning in the spike)
//!
//! See `docs/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md`. In
//! short: `sigstore-verify` (NOT the similarly-named `sigstore-verification`)
//! because its trust root is a required, caller-supplied constructor
//! argument with no network TUF fetch, its `verify()` is synchronous, and its
//! dependency tree carries no `native-tls`/`openssl` — `sigstore-verification`
//! fails on both counts and would violate this crate's `rustls-tls` pin.
//!
//! # Transparency log: kept, not skipped
//!
//! `VerificationPolicy::default()` verifies Rekor transparency-log inclusion
//! proofs. The spike proved real SLSA build-provenance bundles (the kind our
//! release pipeline emits) carry them, so `skip_tlog()` is never called here —
//! doing so would silently discard a real part of the guarantee for no
//! reason.
//!
//! # What `Ok(())` does and does not mean
//!
//! Success means: this bundle is a validly-signed, transparency-logged SLSA
//! build-provenance attestation, issued by GitHub Actions' OIDC identity
//! provider, for the **exact** workflow ref that builds our releases at the
//! **exact** tag requested, and it commits to the **exact** sha256 digest of
//! the archive in hand. It says nothing about whether the source the
//! workflow built from is what a reviewer expects — that is out of scope here,
//! the same way `manifest.rs`'s per-file check says nothing about an active
//! attacker who corrupts both an install dir and its co-located manifest.

use sigstore_trust_root::TrustedRoot;
use sigstore_types::{Bundle, Sha256Hash};
use sigstore_verify::VerificationPolicy;

/// The pinned Sigstore public-good trust root, embedded at compile time. See
/// the module docs for why this file (and not the other root
/// `gh attestation trusted-root` emits) is the one pinned here.
const PINNED_TRUSTED_ROOT_JSON: &str =
    include_str!("../trust/sigstore-public-good-trusted-root.json");

/// The repo whose Actions workflow signs our release artifacts.
const RELEASE_REPO: &str = "tasker-systems/temper";

/// The workflow file that builds release binaries and requests their
/// build-provenance attestation. See `global-constraints.md`'s
/// `build-cli-binaries.yml:177,189` references for the workflow this names.
const RELEASE_WORKFLOW_FILE: &str = "build-cli-binaries.yml";

/// GitHub Actions' OIDC issuer. Every Fulcio certificate it requests carries
/// this URL in the Fulcio issuer extension (OID `1.3.6.1.4.1.57264.1.1`).
const GITHUB_ACTIONS_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Verification failed. The two variants are deliberately distinguishable —
/// **never collapse them into one opaque error** — because their recoveries
/// do not overlap:
///
/// - [`AttestError::TrustRootUnusable`]: something is wrong with the root we
///   embedded, not with the artifact. The pinned trust root failed to parse,
///   or the bundle's certificate chain does not lead to it (an unknown or
///   expired issuer). This binary's pin is stale or broken; the fix ships in
///   a new one — cut a release, or re-run `install.sh` to fetch it.
/// - [`AttestError::NotOurs`]: the root and chain checked out fine, but this
///   specific bundle does not vouch for this specific artifact — wrong
///   signature, wrong workflow identity, wrong digest, or input we could not
///   even parse. The artifact in hand is not one our pinned trust admits.
///
/// Never degrade either variant to a warning: a silent downgrade to
/// "unverified" is exactly the hole this module exists to close.
#[derive(Debug)]
pub enum AttestError {
    /// The embedded trust root is unusable: it failed to parse, or the
    /// bundle's signing certificate does not chain to it.
    TrustRootUnusable(String),
    /// The bundle does not vouch for this artifact under our identity
    /// policy, or the bundle/digest we were given could not be parsed.
    NotOurs(String),
}

impl std::fmt::Display for AttestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttestError::TrustRootUnusable(reason) => write!(
                f,
                "release attestation trust root is unusable ({reason}); this build's \
                 pinned Sigstore trust root may be stale — cut a new release or re-run \
                 install.sh to get a binary with a current one"
            ),
            AttestError::NotOurs(reason) => write!(
                f,
                "release attestation verification failed ({reason}); this artifact is not ours"
            ),
        }
    }
}

impl std::error::Error for AttestError {}

/// Verify that `bundle_json` is a build-provenance attestation, signed by
/// GitHub Actions for **our** release workflow at **exactly** `expected_tag`,
/// covering an artifact whose sha256 digest is `archive_sha256_hex`.
///
/// Verification runs entirely offline against the trust root pinned into this
/// binary (`PINNED_TRUSTED_ROOT_JSON`) — no network access, no TUF fetch.
///
/// `archive_sha256_hex` is the archive's digest only — the ~99 MB archive
/// itself is never read into memory here. The hex string is converted to raw
/// bytes via [`Sha256Hash::from_hex`], whose `Into<Artifact>` conversion
/// produces the digest-only `Artifact::Digest` form (as opposed to
/// `Artifact::Bytes`, which would require the caller to hold the whole
/// archive in memory).
pub fn verify_release_attestation(
    archive_sha256_hex: &str,
    bundle_json: &str,
    expected_tag: &str,
) -> Result<(), AttestError> {
    let trusted_root = TrustedRoot::from_json(PINNED_TRUSTED_ROOT_JSON).map_err(|e| {
        AttestError::TrustRootUnusable(format!("pinned trust root failed to parse: {e}"))
    })?;

    let bundle = Bundle::from_json(bundle_json)
        .map_err(|e| AttestError::NotOurs(format!("attestation bundle is not valid JSON: {e}")))?;

    let digest = Sha256Hash::from_hex(archive_sha256_hex).map_err(|e| {
        AttestError::NotOurs(format!(
            "archive digest is not a valid sha256 hex string: {e}"
        ))
    })?;

    let policy = VerificationPolicy::default()
        .require_issuer(GITHUB_ACTIONS_OIDC_ISSUER)
        .require_identity(expected_identity(expected_tag));

    sigstore_verify::verify(digest, &bundle, &policy, &trusted_root)
        .map(|_| ())
        .map_err(classify_verify_error)
}

/// The exact Fulcio SAN identity our release workflow's certificate carries
/// at a given tag: `https://github.com/<repo>/.github/workflows/<file>@refs/tags/<tag>`.
///
/// This string **is** the security property this module enforces. Get the
/// format wrong and either genuine releases stop verifying, or a wider set of
/// bundles than intended starts passing.
fn expected_identity(tag: &str) -> String {
    format!(
        "https://github.com/{RELEASE_REPO}/.github/workflows/{RELEASE_WORKFLOW_FILE}@refs/tags/{tag}"
    )
}

/// Route a `sigstore-verify` failure into one of our two recovery classes.
///
/// `sigstore-verify` reports everything through a handful of string-carrying
/// variants (mostly [`sigstore_verify::Error::Verification`]), so this
/// classification is necessarily a text match rather than a matched enum
/// variant. Chain-to-root failures ("certificate chain validation failed",
/// which is how webpki's `UnknownIssuer`/expired-anchor errors surface here,
/// and Fulcio-anchor/SCT extraction failures) are about **our** pinned root's
/// material, not this bundle's binding to an artifact — everything else
/// (identity mismatch, issuer mismatch, bad signature, digest mismatch,
/// malformed structure) means this specific bundle does not vouch for this
/// specific artifact.
fn classify_verify_error(err: sigstore_verify::Error) -> AttestError {
    let msg = err.to_string();
    let is_root_problem = msg.contains("certificate chain validation failed")
        || msg.contains("Fulcio certificates")
        || msg.contains("trust anchors")
        || msg.contains(" SCT");
    if is_root_problem {
        AttestError::TrustRootUnusable(msg)
    } else {
        AttestError::NotOurs(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real GitHub build-provenance bundle, from the public `cli/cli`
    /// repo (`gh_2.96.0_macOS_arm64.zip`). Signed for `cli/cli`, not us —
    /// that is the point: it lets us prove both that a genuine bundle
    /// verifies (positive test) and that our identity scoping rejects
    /// bundles for the wrong repo (negative test).
    const FIXTURE_BUNDLE_JSON: &str =
        include_str!("../tests/fixtures/attestation/github-build-provenance-bundle.json");

    /// sha256 of `gh_2.96.0_macOS_arm64.zip`, the artifact the fixture's DSSE
    /// statement lists as its subject.
    const FIXTURE_ARCHIVE_SHA256_HEX: &str =
        "f23a0c37d963aacc3bed703ccbd59b41c5ca22101fab7f00eb2b7cad23aba463";

    /// The real cli/cli bundle verifies against our pinned public-good root
    /// under a permissive (no identity/issuer constraint) policy. This is
    /// what proves the chain, the pinned root, and Rekor inclusion-proof
    /// (transparency log) handling all work end-to-end, offline — the
    /// residual risk the crate-evaluation spike closed.
    #[test]
    fn fixture_verifies_against_pinned_root_with_permissive_policy() {
        let trusted_root =
            TrustedRoot::from_json(PINNED_TRUSTED_ROOT_JSON).expect("pinned trust root parses");
        let bundle = Bundle::from_json(FIXTURE_BUNDLE_JSON).expect("fixture bundle parses");
        let digest =
            Sha256Hash::from_hex(FIXTURE_ARCHIVE_SHA256_HEX).expect("fixture digest is valid hex");

        let result = sigstore_verify::verify(
            digest,
            &bundle,
            &VerificationPolicy::default(),
            &trusted_root,
        );

        assert!(
            result.is_ok(),
            "expected the real cli/cli fixture to verify against the pinned root: {:?}",
            result.err()
        );
    }

    /// THE LOAD-BEARING CASE. The fixture is a genuinely valid, correctly
    /// signed, transparency-logged GitHub build-provenance bundle — just for
    /// `cli/cli`, not `tasker-systems/temper`. A policy scoped to OUR
    /// workflow identity MUST reject it.
    ///
    /// Without this rejection, `verify_release_attestation` would only prove
    /// "signed by GitHub Actions for some repo's release workflow" —
    /// which any public repo running `attest-build-provenance` satisfies —
    /// rather than "signed for THIS repo's release workflow at THIS tag".
    /// That would be barely better than no verification at all: an attacker
    /// (or just a confused build) could hand us any GitHub-attested artifact
    /// from anywhere and it would pass. Asserting the rejection is what
    /// proves the tag-scoped identity check actually narrows trust, rather
    /// than merely parsing the bundle successfully.
    #[test]
    fn wrong_repo_identity_is_rejected() {
        let result =
            verify_release_attestation(FIXTURE_ARCHIVE_SHA256_HEX, FIXTURE_BUNDLE_JSON, "v2.96.0");

        match result {
            Err(AttestError::NotOurs(reason)) => {
                assert!(
                    reason.contains("identity"),
                    "expected an identity-mismatch reason, got: {reason}"
                );
            }
            other => panic!("expected AttestError::NotOurs, got {other:?}"),
        }
    }

    /// The two failure classes must render distinguishably, because their
    /// recoveries do not overlap: a stale/broken pinned root is fixed by
    /// shipping a new binary (`install.sh` is the user-facing recovery path);
    /// a bad signature or wrong identity means the artifact in hand is not
    /// ours. Collapsing them into one opaque message would make either
    /// failure unactionable.
    #[test]
    fn trust_root_and_identity_failures_are_distinguishable() {
        let root_err = classify_verify_error(sigstore_verify::Error::Verification(
            "certificate chain validation failed: UnknownIssuer".to_string(),
        ));
        let identity_err = classify_verify_error(sigstore_verify::Error::Verification(
            "identity mismatch: expected a, got b".to_string(),
        ));

        assert!(matches!(root_err, AttestError::TrustRootUnusable(_)));
        assert!(matches!(identity_err, AttestError::NotOurs(_)));

        let root_msg = root_err.to_string();
        assert!(
            root_msg.contains("install.sh"),
            "a trust-root failure must name install.sh as the recovery: {root_msg}"
        );

        let identity_msg = identity_err.to_string();
        assert!(
            identity_msg.contains("not ours"),
            "an identity/signature failure must say the artifact is not ours: {identity_msg}"
        );
    }

    /// Input we cannot even parse fails closed as `NotOurs` — we cannot vouch
    /// for something we cannot read, and "cannot tell" must not read as
    /// "trust root problem" (which would suggest reinstalling fixes it; it
    /// would not).
    #[test]
    fn malformed_bundle_json_is_rejected() {
        let result = verify_release_attestation(FIXTURE_ARCHIVE_SHA256_HEX, "not json", "v2.96.0");
        assert!(matches!(result, Err(AttestError::NotOurs(_))));
    }

    /// The identity string is the whole security property this module
    /// enforces — pin its exact shape so a future edit that changes the
    /// format is caught here rather than in a confusing verification
    /// failure at release time.
    #[test]
    fn expected_identity_matches_the_release_workflow_ref() {
        assert_eq!(
            expected_identity("v0.3.0"),
            "https://github.com/tasker-systems/temper/.github/workflows/build-cli-binaries.yml@refs/tags/v0.3.0"
        );
    }
}
