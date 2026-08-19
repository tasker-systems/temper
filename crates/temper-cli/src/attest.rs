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
//! See `internal/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md`. In
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
//! provider, for **our** release workflow running on `main`, **triggered by
//! our release-tag chain** (`release-tag.yml` — the chain's entry workflow,
//! pinned inside the signed predicate), and it commits to the **exact** sha256
//! digest of the archive in hand. It says nothing about whether the source the
//! workflow built from is what a reviewer expects — that is out of scope here,
//! the same way `manifest.rs`'s per-file check says nothing about an active
//! attacker who corrupts both an install dir and its co-located manifest.
//!
//! # Why `refs/heads/main`, not `refs/tags/{tag}`
//!
//! The release chain is branch-triggered by construction: `release-tag.yml`
//! fires on a push to `main` (the `VERSION` file), creates and pushes the tag
//! with `GITHUB_TOKEN`, then calls `release.yml` → `build-cli-binaries.yml` via
//! `workflow_call`. A tag pushed with `GITHUB_TOKEN` does not trigger
//! workflows, so `release.yml`'s own `on: push: tags: v*` never fires
//! (`release-tag.yml:53-56` documents this). A reusable workflow called via
//! `workflow_call` inherits the caller's `github.ref`, which is
//! `refs/heads/main` throughout the chain — so the OIDC token's `ref` claim,
//! and therefore the Fulcio cert SAN, carries `@refs/heads/main` for every
//! release. The tag is carried by the archive filename
//! (`temper-v{version}-{target}.tar.gz`) and the manifest's `version` field,
//! not by the cert SAN. The digest match (already enforced below) is what
//! binds to a specific release artifact.
//!
//! # Why the SLSA predicate's workflow path is pinned too
//!
//! The SAN carries `build-cli-binaries.yml`, but `build-cli-binaries.yml`
//! carries its own `workflow_dispatch` trigger (`build-cli-binaries.yml:10-16`,
//! free-string `version`), so a direct dispatch — not via the release-tag
//! chain — would produce an attestation with the same SAN. The SLSA predicate
//! inside the signed DSSE envelope carries
//! `predicate.buildDefinition.externalParameters.workflow.path`, which names
//! the chain's **entry** workflow: `release-tag.yml` for the legitimate
//! chain, `build-cli-binaries.yml` for a direct dispatch. Pinning
//! `release-tag.yml` in the predicate closes the direct-dispatch door: a
//! directly-dispatched build's attestation fails the predicate check even
//! though its SAN would pass. The predicate is signature-covered, so this is
//! a real binding, not a heuristic.

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

/// The ref the release chain runs on. See the module docs for why this is
/// `refs/heads/main` and not `refs/tags/{tag}` — the chain is branch-triggered
/// by construction, and a reusable workflow inherits the caller's `github.ref`.
const RELEASE_WORKFLOW_REF: &str = "refs/heads/main";

/// The chain's entry workflow — the file that fires on a `VERSION`-file push
/// to `main` and calls `release.yml` → `build-cli-binaries.yml`. The SLSA
/// predicate's `externalParameters.workflow.path` names this for a legitimate
/// release, and `build-cli-binaries.yml` for a direct `workflow_dispatch` —
/// pinning `release-tag.yml` here closes the direct-dispatch door. See the
/// module docs and the design spec for the full reasoning.
const RELEASE_CHAIN_ENTRY_WORKFLOW: &str = ".github/workflows/release-tag.yml";

/// The repository URL the SLSA predicate carries for our releases. Belt-and-
/// braces alongside the SAN's `RELEASE_REPO`, but the predicate is a separate
/// signed field so we assert it independently.
const RELEASE_REPOSITORY_URL: &str = "https://github.com/tasker-systems/temper";

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
/// GitHub Actions for **our** release workflow on `main`, **triggered by our
/// release-tag chain**, covering an artifact whose sha256 digest is
/// `archive_sha256_hex`.
///
/// `expected_tag` is carried for caller context and digest binding — it is no
/// longer interpolated into the cert SAN identity, because the release chain
/// is branch-triggered and the SAN carries `@refs/heads/main` for every
/// release (see the module docs). The binding to a *specific* release artifact
/// is via the digest match, already enforced below.
///
/// Verification runs entirely offline against the trust root pinned into this
/// binary (`PINNED_TRUSTED_ROOT_JSON`) — no network access, no TUF fetch.
///
/// Two identity checks run, in order:
/// 1. **Cert SAN** (via `sigstore-verify`'s `require_identity`) — the Fulcio
///    certificate's SAN must be
///    `https://github.com/{RELEASE_REPO}/.github/workflows/{RELEASE_WORKFLOW_FILE}@{RELEASE_WORKFLOW_REF}`.
/// 2. **SLSA predicate workflow path** (custom, after `sigstore-verify`
///    passes) — the signed in-toto statement's
///    `predicate.buildDefinition.externalParameters.workflow.{path,repository}`
///    must be `({RELEASE_CHAIN_ENTRY_WORKFLOW}, {RELEASE_REPOSITORY_URL})`.
///    This closes the direct-dispatch door on `build-cli-binaries.yml`: a
///    directly-dispatched build carries `build-cli-binaries.yml` as its
///    workflow path and fails here.
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
    let _ = expected_tag; // carried for caller context; see the doc above.

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
        .require_identity(expected_identity());

    sigstore_verify::verify(digest, &bundle, &policy, &trusted_root)
        .map_err(classify_verify_error)?;

    // The SAN check above binds to "our release workflow on main." This check
    // binds to "triggered by our release-tag chain" — the SLSA predicate
    // inside the signed envelope names the chain's entry workflow, which is
    // `release-tag.yml` for the legitimate chain and `build-cli-binaries.yml`
    // for a direct dispatch. Without this, a directly-dispatched build's
    // attestation would pass (same SAN) and verify on every consumer path.
    verify_predicate_workflow_path(bundle_json)
}

/// The exact Fulcio SAN identity our release workflow's certificate carries.
/// The release chain is branch-triggered (`release-tag.yml` on push to `main`
/// → `release.yml` → `build-cli-binaries.yml`, all via `workflow_call`), and a
/// reusable workflow inherits the caller's `github.ref` — so the OIDC `ref`
/// claim is `refs/heads/main` for every release, not `refs/tags/{tag}`. See
/// the module docs for the full chain reasoning.
///
/// This string **is** the SAN security property this module enforces. Get the
/// format wrong and either genuine releases stop verifying, or a wider set of
/// bundles than intended starts passing.
fn expected_identity() -> String {
    format!(
        "https://github.com/{RELEASE_REPO}/.github/workflows/{RELEASE_WORKFLOW_FILE}@{RELEASE_WORKFLOW_REF}"
    )
}

/// Assert the SLSA predicate's `externalParameters.workflow.{path,repository}`
/// match our release-tag chain. The predicate is inside the signed DSSE
/// envelope, so it is signature-covered — this is a real binding, not a
/// heuristic. See the module docs and [`verify_release_attestation`] for why
/// this check exists alongside the SAN check.
///
/// Fails closed as [`AttestError::NotOurs`] when the path or repository field
/// is absent or mismatched — never skip. A bundle we cannot read the predicate
/// of is one we cannot vouch for.
fn verify_predicate_workflow_path(bundle_json: &str) -> Result<(), AttestError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let bundle: serde_json::Value = serde_json::from_str(bundle_json)
        .map_err(|e| AttestError::NotOurs(format!("attestation bundle is not valid JSON: {e}")))?;
    let payload_b64 = bundle
        .get("dsseEnvelope")
        .and_then(|env| env.get("payload"))
        .and_then(|p| p.as_str())
        .ok_or_else(|| {
            AttestError::NotOurs("attestation bundle has no dsseEnvelope.payload".to_string())
        })?;
    let decoded = STANDARD.decode(payload_b64).map_err(|e| {
        AttestError::NotOurs(format!("dsseEnvelope.payload is not valid base64: {e}"))
    })?;
    let statement: serde_json::Value = serde_json::from_slice(&decoded).map_err(|e| {
        AttestError::NotOurs(format!(
            "decoded dsseEnvelope payload is not valid JSON: {e}"
        ))
    })?;
    let workflow = statement
        .get("predicate")
        .and_then(|p| p.get("buildDefinition"))
        .and_then(|bd| bd.get("externalParameters"))
        .and_then(|ep| ep.get("workflow"))
        .ok_or_else(|| {
            AttestError::NotOurs(
                "attestation predicate has no buildDefinition.externalParameters.workflow \
                 (cannot determine which workflow triggered this build)"
                    .to_string(),
            )
        })?;

    let path = workflow
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| {
            AttestError::NotOurs(
                "attestation predicate's workflow.path is absent or not a string".to_string(),
            )
        })?;
    let repository = workflow
        .get("repository")
        .and_then(|r| r.as_str())
        .ok_or_else(|| {
            AttestError::NotOurs(
                "attestation predicate's workflow.repository is absent or not a string".to_string(),
            )
        })?;

    if path != RELEASE_CHAIN_ENTRY_WORKFLOW {
        return Err(AttestError::NotOurs(format!(
            "attestation predicate workflow.path is {path:?}, expected \
             {RELEASE_CHAIN_ENTRY_WORKFLOW:?} — this build was not triggered by the release-tag \
             chain (a direct workflow_dispatch on build-cli-binaries.yml carries \
             build-cli-binaries.yml as its path and is rejected here)"
        )));
    }
    if repository != RELEASE_REPOSITORY_URL {
        return Err(AttestError::NotOurs(format!(
            "attestation predicate workflow.repository is {repository:?}, expected \
             {RELEASE_REPOSITORY_URL:?}"
        )));
    }
    Ok(())
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
    use base64::Engine as _;

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
    /// rather than "signed for THIS repo's release workflow on THIS chain".
    /// That would be barely better than no verification at all: an attacker
    /// (or just a confused build) could hand us any GitHub-attested artifact
    /// from anywhere and it would pass. Asserting the rejection is what
    /// proves the identity check actually narrows trust, rather than merely
    /// parsing the bundle successfully.
    ///
    /// The cli/cli fixture fails on the SAN (its cert identity is
    /// `cli/cli`'s `deployment.yml@refs/heads/trunk`, not ours) before the
    /// predicate-path check even runs. The dedicated predicate-path tests
    /// below cover the case where the SAN would pass but the predicate fails.
    #[test]
    fn wrong_repo_identity_is_rejected() {
        let result =
            verify_release_attestation(FIXTURE_ARCHIVE_SHA256_HEX, FIXTURE_BUNDLE_JSON, "v2.96.0");

        match result {
            Err(AttestError::NotOurs(_)) => {}
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

    /// The identity string is the whole SAN security property this module
    /// enforces — pin its exact shape so a future edit that changes the
    /// format is caught here rather than in a confusing verification
    /// failure at release time. The SAN carries `@refs/heads/main`, not
    /// `@refs/tags/{tag}`, because the release chain is branch-triggered
    /// (see the module docs).
    #[test]
    fn expected_identity_matches_the_release_workflow_ref() {
        assert_eq!(
            expected_identity(),
            "https://github.com/tasker-systems/temper/.github/workflows/build-cli-binaries.yml@refs/heads/main"
        );
    }

    // ---- SLSA predicate workflow-path tests ----
    //
    // The SAN check (above) binds to "our release workflow on main." The
    // predicate-path check binds to "triggered by our release-tag chain" —
    // the SLSA predicate inside the signed envelope names the chain's entry
    // workflow, which is `release-tag.yml` for the legitimate chain and
    // `build-cli-binaries.yml` for a direct `workflow_dispatch`. These tests
    // exercise the predicate check in isolation: they feed bundles whose
    // `dsseEnvelope.payload` decodes to a statement carrying a known
    // `predicate.buildDefinition.externalParameters.workflow` object, so the
    // check can be tested without a real signed bundle (the signature is
    // verified by `sigstore-verify` upstream; the predicate check runs only
    // after that passes, so it only ever sees a bundle whose signature is
    // already trusted).

    /// Build a bundle whose `dsseEnvelope.payload` decodes to an in-toto
    /// statement carrying `predicate.buildDefinition.externalParameters.workflow`
    /// with the given `path`, `ref`, and `repository`. Mirrors the real bundle
    /// shape closely enough for `verify_predicate_workflow_path` to exercise
    /// the same base64 + nested-JSON path a genuine bundle takes.
    fn bundle_with_workflow_path(path: &str, ref_: &str, repository: &str) -> String {
        let payload = format!(
            r#"{{"predicateType":"https://slsa.dev/provenance/v1","predicate":{{"buildDefinition":{{"externalParameters":{{"workflow":{{"ref":"{ref_}","repository":"{repository}","path":"{path}"}}}}}}}}}}"#
        );
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload.as_bytes());
        format!(r#"{{"dsseEnvelope":{{"payload":"{payload_b64}"}}}}"#)
    }

    /// A bundle carrying our chain's entry workflow (`release-tag.yml`) and
    /// repository passes the predicate check — this is the shape a legitimate
    /// release attestation carries.
    #[test]
    fn predicate_workflow_path_accepts_release_tag_yml() {
        let bundle = bundle_with_workflow_path(
            RELEASE_CHAIN_ENTRY_WORKFLOW,
            "refs/heads/main",
            RELEASE_REPOSITORY_URL,
        );
        verify_predicate_workflow_path(&bundle)
            .expect("a bundle carrying release-tag.yml + our repo must pass the predicate check");
    }

    /// THE DIRECT-DISPATCH DOOR. A bundle carrying `build-cli-binaries.yml`
    /// as its workflow path — the shape a direct `workflow_dispatch` on
    /// `build-cli-binaries.yml` (the free-string `version` door at
    /// `build-cli-binaries.yml:10-16`) would produce — is rejected. Without
    /// this check, such a bundle would pass the SAN (same
    /// `build-cli-binaries.yml@refs/heads/main` identity) and verify on every
    /// consumer path. This is the case Option 1 (SAN-only) would miss.
    #[test]
    fn predicate_workflow_path_rejects_direct_dispatch_on_build_cli_binaries() {
        let bundle = bundle_with_workflow_path(
            ".github/workflows/build-cli-binaries.yml",
            "refs/heads/main",
            RELEASE_REPOSITORY_URL,
        );
        let err = verify_predicate_workflow_path(&bundle)
            .expect_err("a directly-dispatched build must fail the predicate check");
        let msg = err.to_string();
        assert!(
            msg.contains("release-tag.yml"),
            "the rejection must name the expected chain entry workflow: {msg}"
        );
        assert!(
            msg.contains("build-cli-binaries.yml"),
            "the rejection must name the offending workflow path: {msg}"
        );
        assert!(matches!(err, AttestError::NotOurs(_)));
    }

    /// A bundle carrying a foreign repository is rejected — belt-and-braces
    /// alongside the SAN's `RELEASE_REPO`, but the predicate is a separate
    /// signed field so we assert it independently.
    #[test]
    fn predicate_workflow_path_rejects_foreign_repository() {
        let bundle = bundle_with_workflow_path(
            RELEASE_CHAIN_ENTRY_WORKFLOW,
            "refs/heads/main",
            "https://github.com/cli/cli",
        );
        let err = verify_predicate_workflow_path(&bundle)
            .expect_err("a foreign-repository bundle must fail the predicate check");
        assert!(matches!(err, AttestError::NotOurs(_)));
    }

    /// A bundle whose predicate is missing `buildDefinition.externalParameters.workflow`
    /// entirely fails closed as `NotOurs` — we cannot determine which workflow
    /// triggered this build, and "cannot tell" must never read as "trusted."
    #[test]
    fn predicate_workflow_path_fails_closed_when_workflow_field_absent() {
        let payload = r#"{"predicateType":"https://slsa.dev/provenance/v1","predicate":{"buildDefinition":{"externalParameters":{}}}}"#;
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload.as_bytes());
        let bundle = format!(r#"{{"dsseEnvelope":{{"payload":"{payload_b64}"}}}}"#);
        let err = verify_predicate_workflow_path(&bundle)
            .expect_err("a bundle with no workflow field must fail closed");
        assert!(matches!(err, AttestError::NotOurs(_)));
    }

    /// A bundle with no `dsseEnvelope.payload` at all fails closed — the
    /// predicate check must never skip on a shape it cannot read.
    #[test]
    fn predicate_workflow_path_fails_closed_on_malformed_bundle() {
        let err = verify_predicate_workflow_path("not json")
            .expect_err("malformed bundle JSON must fail closed");
        assert!(matches!(err, AttestError::NotOurs(_)));
    }
}
