//! Fetch a GitHub build-provenance attestation bundle for a release archive
//! digest, and hand it to [`crate::attest::verify_release_attestation`].
//!
//! # Why this exists as its own module, next to `attest.rs`
//!
//! `attest.rs` (Task 9a) verifies a bundle **offline**: it takes a
//! caller-supplied `bundle_json` string and never touches the network. Two
//! call sites now need to *obtain* that bundle from GitHub first — `temper
//! update` (verifying a freshly downloaded archive before installing it) and
//! `temper version --verify --online` (verifying the archive behind the
//! currently installed version). Both must fetch the same way, so the fetch
//! lives here once rather than being copied into each command module.
//!
//! # The API shape (live-probed against `GET
//! /repos/tasker-systems/temper/attestations/sha256:{hex}`)
//!
//! GitHub can record more than one attestation for a single digest — the
//! real response here carried **two** — so picking `[0]` is not a shortcut,
//! it is a bug: whichever attestation kind GitHub happens to list first would
//! silently become "the" attestation. Only an entry whose bundle carries the
//! `https://slsa.dev/provenance/v1` predicate type is the build-provenance
//! attestation `attest::verify_release_attestation` knows how to check, so
//! selection is mandatory (see `select_slsa_provenance_bundle`).
//!
//! Each entry carries `bundle` (inline JSON, possibly `null`) and
//! `bundle_url` (a short-lived presigned URL to the same content). The
//! inline form was observed `null` in practice, so both must be handled: use
//! `bundle` when present, otherwise fetch `bundle_url` (`resolve_bundle_json`).
//! The predicate type itself is not a top-level field — it lives inside the
//! bundle's `dsseEnvelope.payload`, which is base64 and decodes to a JSON
//! in-toto statement carrying `predicateType` (`extract_predicate_type`).
//!
//! # Failure posture
//!
//! [`AttestationVerifyError`] adds a `Network` class on top of
//! [`crate::attest::AttestError`]'s two: a fetch failure (GitHub unreachable,
//! rate-limited, or an unparseable response) says nothing about the artifact
//! — nothing has been checked yet — so it must never render as "this
//! artifact is not ours". Only `TrustRootUnusable` and `NotOurs` (surfaced
//! via [`From<AttestError>`]) carry that verdict, and they stay
//! distinguishable through to the caller exactly as `attest.rs` intends.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;

use crate::attest::{self, AttestError};

/// GitHub's attestations-by-digest endpoint, scoped to this repo.
const GITHUB_ATTESTATIONS_URL_BASE: &str =
    "https://api.github.com/repos/tasker-systems/temper/attestations";

/// The predicate type of a GitHub build-provenance attestation — the only
/// kind [`crate::attest::verify_release_attestation`] knows how to check. A
/// digest can carry other attestation kinds too (GitHub does not guarantee
/// only one), so this is a required selector, not a formality.
pub const SLSA_PROVENANCE_PREDICATE_TYPE: &str = "https://slsa.dev/provenance/v1";

/// Failure fetching, selecting, or verifying a release attestation. Three
/// classes, deliberately distinguishable — their recoveries do not overlap,
/// mirroring [`AttestError`]'s own two-way split plus one more:
///
/// - [`Network`](AttestationVerifyError::Network): could not even reach
///   GitHub (or GitHub itself hiccuped, or answered with something we could
///   not parse). This says nothing about the artifact — nothing has been
///   checked yet. A dropped wifi connection must never read as "you've been
///   attacked".
/// - [`TrustRootUnusable`](AttestationVerifyError::TrustRootUnusable): see
///   [`AttestError::TrustRootUnusable`] — the fix ships in a new binary, or a
///   re-run of `install.sh`.
/// - [`NotOurs`](AttestationVerifyError::NotOurs): see
///   [`AttestError::NotOurs`] — GitHub answered and the answer does not vouch
///   for this artifact: no attestations at all, none carrying the SLSA
///   build-provenance predicate, or the bundle's own verification failed.
#[derive(Debug)]
pub enum AttestationVerifyError {
    /// Could not reach GitHub, GitHub itself errored, or its response could
    /// not be parsed. Not a verdict on the artifact.
    Network(String),
    /// The embedded trust root is unusable. Same meaning as
    /// [`AttestError::TrustRootUnusable`].
    TrustRootUnusable(String),
    /// GitHub's answer does not vouch for this artifact. Same meaning as
    /// [`AttestError::NotOurs`], plus the fetch-side cases (no attestations,
    /// no SLSA-provenance predicate among them) that never reach `attest.rs`
    /// at all.
    NotOurs(String),
}

impl std::fmt::Display for AttestationVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttestationVerifyError::Network(reason) => write!(
                f,
                "could not fetch the release attestation ({reason}); your install is unchanged \
                 — this does not mean the artifact is untrusted, only that the check could not \
                 run. Retry, or check your network connection."
            ),
            AttestationVerifyError::TrustRootUnusable(reason) => write!(
                f,
                "release attestation trust root is unusable ({reason}); this build's pinned \
                 Sigstore trust root may be stale — cut a new release or re-run install.sh to \
                 get a binary with a current one"
            ),
            AttestationVerifyError::NotOurs(reason) => write!(
                f,
                "release attestation verification failed ({reason}); this artifact is not ours"
            ),
        }
    }
}

impl std::error::Error for AttestationVerifyError {}

impl From<AttestError> for AttestationVerifyError {
    fn from(e: AttestError) -> Self {
        match e {
            AttestError::TrustRootUnusable(msg) => AttestationVerifyError::TrustRootUnusable(msg),
            AttestError::NotOurs(msg) => AttestationVerifyError::NotOurs(msg),
        }
    }
}

/// The temper-cli HTTP client posture every GitHub call in this crate shares
/// — a `temper-cli/{version}` user agent and bounded connect/total timeouts
/// (reqwest has no default timeout, so a black-holed network must not hang
/// forever). `update.rs::resolve_latest_tag` established this posture first;
/// every later call site (archive/manifest download, this module's
/// attestation lookup, `version.rs`'s manifest re-fetch) builds its client
/// through this one function so the crate never carries a second HTTP client
/// configuration. Returns a plain `ClientBuilder` (not yet `.build()`-ed) so
/// each caller can map the build failure into whatever error type its own
/// call site uses.
pub fn release_http_client_builder(version: &str) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(format!("temper-cli/{version}"))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
}

/// One entry in GitHub's `GET .../attestations/sha256:{digest}` response.
/// Extra fields GitHub includes (`initiator`, `repository_id`) are ignored —
/// serde drops unknown fields by default when `deny_unknown_fields` isn't
/// set.
#[derive(Debug, Deserialize)]
struct AttestationRecord {
    /// Inline bundle JSON. Observed `null` in practice — do not assume it is
    /// always populated.
    bundle: Option<serde_json::Value>,
    /// A short-lived presigned URL to the same bundle content, present when
    /// `bundle` is not.
    bundle_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AttestationsResponse {
    attestations: Vec<AttestationRecord>,
}

/// Fetch every attestation GitHub has recorded for `digest_hex` on this repo.
/// A 404 means GitHub recorded none for this digest — a definitive "this
/// artifact has no attestation", not a transient failure, so it renders
/// [`AttestationVerifyError::NotOurs`]. A 403 is (per `update.rs`'s existing
/// convention for the same GitHub API surface) almost always the
/// unauthenticated rate limit, so it renders [`AttestationVerifyError::Network`]
/// rather than a bare "forbidden" that reads like an auth wall.
async fn fetch_attestation_records(
    client: &reqwest::Client,
    digest_hex: &str,
) -> Result<Vec<AttestationRecord>, AttestationVerifyError> {
    let url = format!("{GITHUB_ATTESTATIONS_URL_BASE}/sha256:{digest_hex}");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| {
            AttestationVerifyError::Network(format!("fetching attestations for {digest_hex}: {e}"))
        })?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AttestationVerifyError::NotOurs(format!(
            "GitHub has no attestations recorded for digest {digest_hex}"
        )));
    }
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(AttestationVerifyError::Network(
            "GitHub API rate-limited or forbidden (HTTP 403) while fetching attestations; your \
             install is unchanged — retry in a few minutes"
                .to_string(),
        ));
    }
    if !resp.status().is_success() {
        return Err(AttestationVerifyError::Network(format!(
            "GitHub attestations API returned {}",
            resp.status()
        )));
    }

    let parsed: AttestationsResponse = resp.json().await.map_err(|e| {
        AttestationVerifyError::Network(format!("parsing attestations response JSON: {e}"))
    })?;
    if parsed.attestations.is_empty() {
        return Err(AttestationVerifyError::NotOurs(format!(
            "GitHub returned zero attestations for digest {digest_hex}"
        )));
    }
    Ok(parsed.attestations)
}

/// Resolve one attestation record to its raw bundle JSON. The inline
/// `bundle` wins when present — no network call is made in that case at
/// all; `bundle_url` is only followed when `bundle` is absent, matching the
/// live-probed shape where one was `null` and the other carried the
/// presigned URL.
async fn resolve_bundle_json(
    client: &reqwest::Client,
    record: &AttestationRecord,
) -> Result<String, AttestationVerifyError> {
    if let Some(bundle) = &record.bundle {
        return serde_json::to_string(bundle).map_err(|e| {
            AttestationVerifyError::NotOurs(format!(
                "inline attestation bundle is not serializable JSON: {e}"
            ))
        });
    }
    let Some(url) = &record.bundle_url else {
        return Err(AttestationVerifyError::NotOurs(
            "attestation record carries neither an inline bundle nor a bundle_url".to_string(),
        ));
    };
    let resp = client.get(url).send().await.map_err(|e| {
        AttestationVerifyError::Network(format!("fetching attestation bundle_url: {e}"))
    })?;
    if !resp.status().is_success() {
        return Err(AttestationVerifyError::Network(format!(
            "fetching attestation bundle_url returned HTTP {}",
            resp.status()
        )));
    }
    resp.text().await.map_err(|e| {
        AttestationVerifyError::Network(format!("reading attestation bundle_url body: {e}"))
    })
}

/// Extract the in-toto `predicateType` from a bundle's
/// `dsseEnvelope.payload` (base64-encoded JSON). Pure, and the load-bearing
/// piece of predicate-type selection: any parse failure at any stage is
/// `NotOurs` — we cannot vouch for a bundle we cannot read, and "cannot
/// tell" here means "does not select as the provenance bundle", not "the
/// trust root is broken".
fn extract_predicate_type(bundle_json: &str) -> Result<String, AttestationVerifyError> {
    let bundle: serde_json::Value = serde_json::from_str(bundle_json).map_err(|e| {
        AttestationVerifyError::NotOurs(format!("attestation bundle is not valid JSON: {e}"))
    })?;
    let payload_b64 = bundle
        .get("dsseEnvelope")
        .and_then(|env| env.get("payload"))
        .and_then(|p| p.as_str())
        .ok_or_else(|| {
            AttestationVerifyError::NotOurs(
                "attestation bundle has no dsseEnvelope.payload".to_string(),
            )
        })?;
    let decoded = STANDARD.decode(payload_b64).map_err(|e| {
        AttestationVerifyError::NotOurs(format!("dsseEnvelope.payload is not valid base64: {e}"))
    })?;
    let statement: serde_json::Value = serde_json::from_slice(&decoded).map_err(|e| {
        AttestationVerifyError::NotOurs(format!(
            "decoded dsseEnvelope payload is not valid JSON: {e}"
        ))
    })?;
    statement
        .get("predicateType")
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            AttestationVerifyError::NotOurs(
                "decoded attestation payload has no predicateType".to_string(),
            )
        })
}

/// Select the one bundle, among possibly several, whose predicate type is
/// [`SLSA_PROVENANCE_PREDICATE_TYPE`]. **This is not `bundles[0]`** — a
/// digest can carry more than one attestation kind, and taking the first
/// unconditionally would silently trust whichever kind GitHub happened to
/// list first. A bundle that fails to parse is skipped (it simply does not
/// match), not treated as fatal — a malformed *other* attestation must not
/// prevent a valid SLSA one elsewhere in the list from being found.
fn select_slsa_provenance_bundle(bundles: &[String]) -> Result<&str, AttestationVerifyError> {
    for bundle in bundles {
        if let Ok(predicate_type) = extract_predicate_type(bundle) {
            if predicate_type == SLSA_PROVENANCE_PREDICATE_TYPE {
                return Ok(bundle);
            }
        }
    }
    Err(AttestationVerifyError::NotOurs(format!(
        "none of the {} attestation(s) returned for this digest carry the SLSA build-provenance \
         predicate ({SLSA_PROVENANCE_PREDICATE_TYPE})",
        bundles.len()
    )))
}

/// Fetch every attestation for `digest_hex`, resolve each to its bundle JSON,
/// and return the one carrying the SLSA build-provenance predicate.
pub async fn fetch_release_attestation_bundle(
    client: &reqwest::Client,
    digest_hex: &str,
) -> Result<String, AttestationVerifyError> {
    let records = fetch_attestation_records(client, digest_hex).await?;
    let mut bundles = Vec::with_capacity(records.len());
    for record in &records {
        bundles.push(resolve_bundle_json(client, record).await?);
    }
    select_slsa_provenance_bundle(&bundles).map(str::to_string)
}

/// Fetch the SLSA build-provenance attestation for `digest_hex` and verify it
/// against our pinned trust root for `tag` — the single reusable helper both
/// `temper update` and `temper version --verify --online` call. There is no
/// bypass: a caller either gets `Ok(())` (verified) or one of
/// [`AttestationVerifyError`]'s three classes.
pub async fn verify_release_attestation_online(
    client: &reqwest::Client,
    digest_hex: &str,
    tag: &str,
) -> Result<(), AttestationVerifyError> {
    let bundle_json = fetch_release_attestation_bundle(client, digest_hex).await?;
    attest::verify_release_attestation(digest_hex, &bundle_json, tag).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal bundle whose `dsseEnvelope.payload` decodes to a JSON
    /// in-toto statement carrying `predicate_type`. Mirrors the real bundle
    /// shape closely enough for `extract_predicate_type` to exercise the
    /// same base64 + nested-JSON path a genuine bundle takes.
    fn bundle_with_predicate(predicate_type: &str) -> String {
        let payload = format!(r#"{{"predicateType":"{predicate_type}"}}"#);
        let payload_b64 = STANDARD.encode(payload.as_bytes());
        format!(r#"{{"dsseEnvelope":{{"payload":"{payload_b64}"}}}}"#)
    }

    #[test]
    fn extract_predicate_type_reads_the_dsse_envelope_payload() {
        let bundle = bundle_with_predicate(SLSA_PROVENANCE_PREDICATE_TYPE);
        assert_eq!(
            extract_predicate_type(&bundle).unwrap(),
            SLSA_PROVENANCE_PREDICATE_TYPE
        );
    }

    #[test]
    fn extract_predicate_type_rejects_malformed_bundle_json() {
        let err = extract_predicate_type("not json").unwrap_err();
        assert!(matches!(err, AttestationVerifyError::NotOurs(_)));
    }

    #[test]
    fn extract_predicate_type_rejects_missing_dsse_envelope() {
        let err = extract_predicate_type(r#"{"somethingElse": true}"#).unwrap_err();
        assert!(matches!(err, AttestationVerifyError::NotOurs(_)));
    }

    /// THE LOAD-BEARING CASE. GitHub returned two attestations for one
    /// digest; the SLSA build-provenance bundle is deliberately placed
    /// **second** in the list. If selection took `bundles[0]` unconditionally
    /// (or otherwise picked position over predicate type), this test fails.
    #[test]
    fn select_slsa_provenance_bundle_picks_the_slsa_one_not_the_first() {
        let bundles = vec![
            bundle_with_predicate("https://example.com/some-other-attestation/v1"),
            bundle_with_predicate(SLSA_PROVENANCE_PREDICATE_TYPE),
        ];
        let selected = select_slsa_provenance_bundle(&bundles).expect("a match exists");
        assert_eq!(selected, bundles[1]);
        assert_ne!(selected, bundles[0]);
    }

    /// Order independence: the SLSA bundle is found even when it's first —
    /// selection isn't accidentally "always pick index 1".
    #[test]
    fn select_slsa_provenance_bundle_finds_it_first_too() {
        let bundles = vec![
            bundle_with_predicate(SLSA_PROVENANCE_PREDICATE_TYPE),
            bundle_with_predicate("https://example.com/some-other-attestation/v1"),
        ];
        let selected = select_slsa_provenance_bundle(&bundles).expect("a match exists");
        assert_eq!(selected, bundles[0]);
    }

    /// A digest whose attestations are all non-SLSA predicates has nothing to
    /// select — this must fail as `NotOurs`, never silently pick a wrong
    /// predicate type.
    #[test]
    fn select_slsa_provenance_bundle_errors_when_none_match() {
        let bundles = vec![bundle_with_predicate(
            "https://example.com/some-other-attestation/v1",
        )];
        let err = select_slsa_provenance_bundle(&bundles).unwrap_err();
        assert!(matches!(err, AttestationVerifyError::NotOurs(_)));
    }

    /// A bundle that fails to parse is skipped, not fatal — a malformed
    /// *other* entry must not hide a valid SLSA one elsewhere in the list.
    #[test]
    fn select_slsa_provenance_bundle_skips_unparseable_entries() {
        let bundles = vec![
            "not json at all".to_string(),
            bundle_with_predicate(SLSA_PROVENANCE_PREDICATE_TYPE),
        ];
        let selected = select_slsa_provenance_bundle(&bundles).expect("a match exists");
        assert_eq!(selected, bundles[1]);
    }

    /// Inline `bundle` wins over `bundle_url` when both are present — and
    /// critically, no network call happens in that case: `bundle_url` here
    /// points at a URL that would fail if ever actually fetched, proving the
    /// inline path short-circuits before any request is made.
    #[tokio::test]
    async fn resolve_bundle_json_prefers_inline_bundle_over_bundle_url() {
        let client = reqwest::Client::new();
        let inline = serde_json::json!({"dsseEnvelope": {"payload": "aGVsbG8="}});
        let record = AttestationRecord {
            bundle: Some(inline.clone()),
            bundle_url: Some("http://127.0.0.1:1/unreachable".to_string()),
        };
        let resolved = resolve_bundle_json(&client, &record)
            .await
            .expect("inline bundle resolves without touching bundle_url");
        let expected = serde_json::to_string(&inline).unwrap();
        assert_eq!(resolved, expected);
    }

    /// Neither `bundle` nor `bundle_url` present is a data-shape failure —
    /// `NotOurs`, not a network problem (nothing was fetched).
    #[tokio::test]
    async fn resolve_bundle_json_errors_when_neither_present() {
        let client = reqwest::Client::new();
        let record = AttestationRecord {
            bundle: None,
            bundle_url: None,
        };
        let err = resolve_bundle_json(&client, &record).await.unwrap_err();
        assert!(matches!(err, AttestationVerifyError::NotOurs(_)));
    }

    /// The three failure classes must render distinguishably — this is the
    /// property the whole module exists to preserve. A network failure must
    /// NOT say "not ours" (that would tell the user they've been attacked
    /// when their wifi dropped); a trust-root failure must name `install.sh`;
    /// an identity/signature failure must say the artifact is not ours.
    #[test]
    fn the_three_failure_classes_render_distinguishably() {
        let network = AttestationVerifyError::Network("connection refused".to_string());
        let trust_root = AttestationVerifyError::TrustRootUnusable("bad root".to_string());
        let not_ours = AttestationVerifyError::NotOurs("wrong identity".to_string());

        let network_msg = network.to_string();
        assert!(
            !network_msg.contains("not ours"),
            "a network failure must not read as 'artifact is not ours': {network_msg}"
        );

        let trust_root_msg = trust_root.to_string();
        assert!(
            trust_root_msg.contains("install.sh"),
            "a trust-root failure must name install.sh as the recovery: {trust_root_msg}"
        );

        let not_ours_msg = not_ours.to_string();
        assert!(
            not_ours_msg.contains("not ours"),
            "an identity/signature failure must say the artifact is not ours: {not_ours_msg}"
        );
    }

    /// `From<AttestError>` preserves each variant's class rather than
    /// collapsing both into one — the whole point of keeping the two
    /// upstream classes alive through this module's third (`Network`).
    #[test]
    fn attest_error_conversion_preserves_the_class() {
        let trust_root: AttestationVerifyError =
            AttestError::TrustRootUnusable("x".to_string()).into();
        assert!(matches!(
            trust_root,
            AttestationVerifyError::TrustRootUnusable(_)
        ));

        let not_ours: AttestationVerifyError = AttestError::NotOurs("y".to_string()).into();
        assert!(matches!(not_ours, AttestationVerifyError::NotOurs(_)));
    }

    /// The HTTP client posture is buildable and carries the expected user
    /// agent — a cheap guard against a typo in the format string silently
    /// shipping an empty/garbled User-Agent (GitHub's API rejects requests
    /// without one).
    #[test]
    fn release_http_client_builder_produces_a_usable_client() {
        let client = release_http_client_builder("9.9.9").build();
        assert!(client.is_ok(), "client should build: {:?}", client.err());
    }
}
