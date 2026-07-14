//! The Vercel Connect adapter behind the broker seam.
//!
//! `mint` is two plain HTTPS hops (a static access token buys a project OIDC
//! token, which mints the provider token); `verify_inbound` is an RS256/JWKS
//! check with an anti-decoy `client_id` assertion. Both were pinned by live
//! probes, not docs — see the design spec and the vault research.

use super::{
    BrokerError, BrokerToken, CredentialBroker, InboundRequest, MintRequest, MintSubject, Minted,
    MintedReach, VerifiedInbound,
};
use crate::state::JwksKeyStore;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use jsonwebtoken::decode;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

/// A path segment keeps the RFC 3986 unreserved set (`A-Za-z0-9-._~`) and encodes
/// everything else — crucially `/` → `%2F`. `NON_ALPHANUMERIC` alone would
/// over-encode `-` and `.`, which appear in connector uids (`mcp.linear.app/x`).
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Configuration for the Vercel Connect broker. The access token is the only
/// secret; the rest are Vercel identifiers. All live in the deployment's env
/// (encrypted for the secret), self-hosted operators set their own — nothing is
/// hardcoded.
#[derive(Clone)]
pub struct VercelConnectConfig {
    /// A Vercel access token (buys the project OIDC token in mint hop 1).
    pub access_token: String,
    pub project_id: String,
    pub team_id: String,
    /// The Vercel team slug — determines the OIDC issuer/audience/JWKS in
    /// team-issuer mode (`https://oidc.vercel.com/<slug>`).
    pub team_slug: String,
}

impl std::fmt::Debug for VercelConnectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never Debug the access token.
        f.debug_struct("VercelConnectConfig")
            .field("project_id", &self.project_id)
            .field("team_id", &self.team_id)
            .field("team_slug", &self.team_slug)
            .finish_non_exhaustive()
    }
}

/// The Vercel Connect adapter. Holds the mint credentials and a JWKS store
/// pointed at the Vercel team issuer for inbound verification.
pub struct VercelConnectBroker {
    http: reqwest::Client,
    config: VercelConnectConfig,
    jwks: JwksKeyStore,
    expected_iss: String,
    expected_aud: String,
}

impl std::fmt::Debug for VercelConnectBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VercelConnectBroker")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl VercelConnectBroker {
    pub fn new(config: VercelConnectConfig) -> Self {
        let expected_iss = format!("https://oidc.vercel.com/{}", config.team_slug);
        let expected_aud = format!("https://vercel.com/{}", config.team_slug);
        let jwks = JwksKeyStore::new(format!(
            "https://oidc.vercel.com/{}/.well-known/jwks",
            config.team_slug
        ));
        Self {
            http: reqwest::Client::new(),
            config,
            jwks,
            expected_iss,
            expected_aud,
        }
    }
}

/// Verify a forwarded webhook: pull the attestation bearer, verify its signature
/// and standard claims against the Vercel JWKS, then apply the Connect-specific
/// claim gate ([`verify_attestation_claims`]). Split out from the trait method so
/// it can be tested network-free with [`JwksKeyStore::with_static_key`].
async fn verify_inbound_impl(
    jwks: &JwksKeyStore,
    expected_iss: &str,
    expected_aud: &str,
    req: InboundRequest<'_>,
) -> Result<VerifiedInbound, BrokerError> {
    // 1. Pull the attestation bearer. This is the `Authorization` header — NOT
    //    `x-vercel-oidc-token`, which is the receiver's own ambient identity.
    let raw = req
        .authorization
        .ok_or_else(|| BrokerError::Verification("no Authorization header".into()))?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw)
        .trim();

    // 2. Fetch the JWKS key + its algorithm (same seam the auth middleware uses).
    let vk = jwks
        .get_decoding_key()
        .await
        .map_err(|e| BrokerError::Transport(format!("JWKS unavailable: {e}")))?;

    // 3. Verify signature + standard claims. `validation` carries the hard-won
    //    `set_required_spec_claims` fix, so a token missing `exp`/`iss`/`aud` is
    //    refused rather than silently accepted.
    let validation = jwks.validation(expected_iss, expected_aud, vk.algorithm);
    let data = decode::<serde_json::Value>(token, &vk.key, &validation)
        .map_err(|e| BrokerError::Verification(format!("attestation signature/claims: {e}")))?;

    // 4. Apply the Connect-specific gate: the anti-decoy `client_id` and the
    //    signed `trigger` claim.
    let trigger = verify_attestation_claims(&data.claims, expected_iss, expected_aud)?;

    Ok(VerifiedInbound {
        provider: trigger.provider,
        connector_uid: trigger.connector_uid,
        connector_id: trigger.connector_id,
        payload: req.body.to_vec(),
    })
}

/// Build the Connect mint URL. The connector uid is **one url-encoded path
/// segment** (`github/acme` → `github%2Facme`) — naive interpolation produces a
/// wrong route. This gotcha cost real time in the probe.
fn connect_token_url(connector: &str) -> String {
    format!(
        "https://api.vercel.com/v1/connect/token/{}",
        utf8_percent_encode(connector, SEGMENT)
    )
}

/// Map a non-success mint response to the right error variant. A connector that
/// needs consent is not a rejected credential, and a caller must tell them apart.
fn map_mint_error(status: u16, body: &str) -> BrokerError {
    let needs_consent = body.contains("user_authorization_required")
        || body.contains("client_installation_required")
        || body.contains("connector_installation_required");
    if needs_consent {
        // The authorize URL, when present, is the recovery path; surface it.
        let url = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.get("authorizationUrl")
                    .or_else(|| v.pointer("/error/authorizationUrl"))
                    .and_then(|u| u.as_str())
                    .map(str::to_string)
            });
        return BrokerError::NeedsConsent { authorize_url: url };
    }
    match status {
        400..=499 => BrokerError::Unauthorized(format!("broker returned {status}: {body}")),
        _ => BrokerError::Transport(format!("broker returned {status}: {body}")),
    }
}

impl VercelConnectBroker {
    /// The two-hop mint: a static access token buys a project OIDC token, which
    /// mints the provider token. Both hops need `Content-Type: application/json`
    /// even with an empty body, or Vercel returns HTTP 415.
    ///
    /// Not unit-tested (it is live-network glue over tested pure pieces —
    /// [`connect_token_url`], [`map_mint_error`], [`parse_mint_response`]); it is
    /// exercised end-to-end against live Vercel and, at the service layer, through
    /// the [`super::FakeBroker`]. A future hardening caches both tokens (Connect
    /// bills per request) and, in a Vercel Function, could skip hop 1 by using the
    /// ambient `x-vercel-oidc-token` header.
    async fn mint_impl(&self, req: MintRequest<'_>) -> Result<Minted, BrokerError> {
        let oidc = self.fetch_project_oidc_token().await?;
        let url = connect_token_url(&req.credential.connector);

        let subject = match req.subject {
            MintSubject::App => serde_json::json!({ "type": "app" }),
        };
        let mut body = serde_json::json!({ "subject": subject });
        if !req.scopes.is_empty() {
            body["scopes"] = serde_json::json!(req.scopes);
        }
        if let Some(installation) = &req.credential.installation {
            body["installationId"] = serde_json::json!(installation);
        }

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&oidc)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| BrokerError::Transport(format!("mint request failed: {e}")))?;

        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| BrokerError::Transport(format!("reading mint response: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(map_mint_error(status, &text));
        }
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| BrokerError::Malformed(format!("mint response not JSON: {e}")))?;
        parse_mint_response(&json)
    }

    /// Hop 1: a static Vercel access token buys a short-lived project OIDC token.
    /// `POST /v1/projects/{id}/token` needs a JSON content-type even though the
    /// body is empty (else HTTP 415).
    async fn fetch_project_oidc_token(&self) -> Result<String, BrokerError> {
        let url = format!(
            "https://api.vercel.com/v1/projects/{}/token?source=vercel-oidc-refresh&teamId={}",
            self.config.project_id, self.config.team_id
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.access_token)
            .header("Accept", "application/json")
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| BrokerError::Transport(format!("OIDC request failed: {e}")))?;

        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| BrokerError::Transport(format!("reading OIDC response: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(map_mint_error(status, &text));
        }
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::to_string))
            .ok_or_else(|| BrokerError::Malformed("OIDC response missing `token`".into()))
    }
}

#[async_trait]
impl CredentialBroker for VercelConnectBroker {
    async fn mint(&self, req: MintRequest<'_>) -> Result<Minted, BrokerError> {
        self.mint_impl(req).await
    }

    async fn verify_inbound(
        &self,
        req: InboundRequest<'_>,
    ) -> Result<VerifiedInbound, BrokerError> {
        verify_inbound_impl(&self.jwks, &self.expected_iss, &self.expected_aud, req).await
    }
}

#[cfg(test)]
mod mint_helper_tests {
    use super::*;

    #[test]
    fn connector_uid_is_one_encoded_path_segment() {
        // The slash becomes %2F; unreserved chars (-, .) are preserved.
        assert_eq!(
            connect_token_url("github/temper-probe"),
            "https://api.vercel.com/v1/connect/token/github%2Ftemper-probe"
        );
        assert_eq!(
            connect_token_url("mcp.linear.app/acme"),
            "https://api.vercel.com/v1/connect/token/mcp.linear.app%2Facme"
        );
    }

    #[test]
    fn a_consent_needed_response_maps_to_needs_consent() {
        let err = map_mint_error(
            403,
            r#"{"error":{"code":"user_authorization_required","authorizationUrl":"https://vercel.com/consent"}}"#,
        );
        match err {
            BrokerError::NeedsConsent { authorize_url } => {
                assert_eq!(authorize_url.as_deref(), Some("https://vercel.com/consent"));
            }
            other => panic!("expected NeedsConsent, got {other:?}"),
        }
    }

    #[test]
    fn a_rejected_credential_maps_to_unauthorized_not_transport() {
        assert!(matches!(
            map_mint_error(401, r#"{"error":{"code":"forbidden"}}"#),
            BrokerError::Unauthorized(_)
        ));
    }

    #[test]
    fn a_server_error_maps_to_transport() {
        assert!(matches!(
            map_mint_error(503, "upstream unavailable"),
            BrokerError::Transport(_)
        ));
    }
}

#[cfg(test)]
mod verify_inbound_tests {
    use super::*;
    use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};

    const ED_PRIV: &str = "-----BEGIN PRIVATE KEY-----\n\
        MC4CAQAwBQYDK2VwBCIEIMBUy9dWl8ECx1v9KN+aoEl/fI80u7Qcv9F8OTVxWW0G\n\
        -----END PRIVATE KEY-----\n";
    const ED_PUB: &str = "-----BEGIN PUBLIC KEY-----\n\
        MCowBQYDK2VwAyEAcCE6sWGL6rcfOATmlUSiuWLQAl+hpPAPp/aTR1yxqdc=\n\
        -----END PUBLIC KEY-----\n";

    const ISS: &str = "https://oidc.vercel.com/acme";
    const AUD: &str = "https://vercel.com/acme";

    fn keys() -> (EncodingKey, DecodingKey) {
        (
            EncodingKey::from_ed_pem(ED_PRIV.as_bytes()).expect("valid ed priv"),
            DecodingKey::from_ed_pem(ED_PUB.as_bytes()).expect("valid ed pub"),
        )
    }

    fn store() -> JwksKeyStore {
        let (_, dec) = keys();
        JwksKeyStore::with_static_key(dec, Algorithm::EdDSA)
    }

    fn sign(claims: &serde_json::Value) -> String {
        let (enc, _) = keys();
        encode(&Header::new(Algorithm::EdDSA), claims, &enc).expect("sign")
    }

    fn valid_attestation() -> serde_json::Value {
        serde_json::json!({
            "iss": ISS,
            "aud": AUD,
            "sub": "owner:acme:project:temper-api:environment:production",
            "client_id": "api-connex",
            "exp": 9_999_999_999_i64,
            "trigger": { "id": "scl_abc", "uid": "slack/acme", "type": "slack", "service": "slack" }
        })
    }

    #[tokio::test]
    async fn verifies_a_signed_attestation_and_returns_the_payload_and_trigger() {
        let token = sign(&valid_attestation());
        let authz = format!("Bearer {token}");
        let req = InboundRequest {
            authorization: Some(&authz),
            body: b"{\"event\":\"hi\"}",
        };
        let v = verify_inbound_impl(&store(), ISS, AUD, req)
            .await
            .expect("verify");
        assert_eq!(v.provider, "slack");
        assert_eq!(v.connector_uid, "slack/acme");
        assert_eq!(v.connector_id, "scl_abc");
        assert_eq!(v.payload, b"{\"event\":\"hi\"}");
    }

    #[tokio::test]
    async fn rejects_a_missing_authorization_header() {
        let req = InboundRequest {
            authorization: None,
            body: b"{}",
        };
        let err = verify_inbound_impl(&store(), ISS, AUD, req)
            .await
            .unwrap_err();
        assert!(matches!(err, BrokerError::Verification(_)));
    }

    #[tokio::test]
    async fn rejects_a_bad_signature() {
        // A token signed by a DIFFERENT key must fail signature verification.
        let other_priv = "-----BEGIN PRIVATE KEY-----\n\
            MC4CAQAwBQYDK2VwBCIEIEGD8kZ8y1sXZ8sQpQ0oT8yJZ0k8k0k0k0k0k0k0k0k\n\
            -----END PRIVATE KEY-----\n";
        // If the alternate key is unusable, fall back to tampering the token.
        let token = match EncodingKey::from_ed_pem(other_priv.as_bytes()) {
            Ok(enc) => encode(&Header::new(Algorithm::EdDSA), &valid_attestation(), &enc)
                .expect("sign with other key"),
            Err(_) => {
                let mut t = sign(&valid_attestation());
                t.push_str("tampered");
                t
            }
        };
        let authz = format!("Bearer {token}");
        let req = InboundRequest {
            authorization: Some(&authz),
            body: b"{}",
        };
        let err = verify_inbound_impl(&store(), ISS, AUD, req)
            .await
            .unwrap_err();
        assert!(matches!(err, BrokerError::Verification(_)));
    }
}

/// Parse a Vercel Connect mint response (`POST /v1/connect/token/{connector}`)
/// into a typed [`Minted`]. The response shape (captured live):
///
/// ```json
/// { "token": "…", "tokenId": "stk_…", "expiresAt": 1784065923026,
///   "connector": {…}, "installationId": "…",
///   "metadata": { "permissions": {…}, "repository_selection": "all" } }
/// ```
///
/// `expiresAt` is milliseconds since the epoch. `metadata` is the reach the
/// provider actually granted.
pub(crate) fn parse_mint_response(body: &serde_json::Value) -> Result<Minted, BrokerError> {
    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BrokerError::Malformed("mint response missing `token`".into()))?;

    let expires_ms = body
        .get("expiresAt")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| BrokerError::Malformed("mint response missing `expiresAt`".into()))?;
    let expires_at: DateTime<Utc> = DateTime::from_timestamp_millis(expires_ms)
        .ok_or_else(|| BrokerError::Malformed(format!("`expiresAt` out of range: {expires_ms}")))?;

    // `metadata` is the provider-shaped reach. Absent ⇒ an empty object rather
    // than an error: not every provider reports permissions, and its absence is
    // "no reach declared," not a malformed response.
    let reach = MintedReach {
        raw: body
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    };

    Ok(Minted {
        token: BrokerToken::new(token.to_string()),
        expires_at,
        reach,
    })
}

/// The `client_id` a genuine Connect-forwarded attestation carries. It is the
/// **only** claim (besides the signed `trigger`) that distinguishes the
/// attestation from the receiving deployment's own ambient OIDC token — which is
/// present on every inbound request and names the same project/iss/aud. Verifying
/// iss/aud/sub alone is NOT sufficient; the ambient token passes it. This check
/// is what stops the deployment's own identity token from forging a webhook.
const CONNECT_CLIENT_ID: &str = "api-connex";

/// The signed connector identity extracted from a verified attestation, before
/// the (already-authenticated) payload is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedTrigger {
    pub provider: String,
    pub connector_uid: String,
    pub connector_id: String,
}

/// Validate the *claims* of a Connect attestation (signature already verified by
/// the caller against the JWKS) and extract the signed connector identity.
///
/// Enforces, in order: issuer, audience, the anti-decoy `client_id`, and the
/// presence of a signed `trigger` claim. The `trigger` is read from the JWT —
/// never from the unsigned `x-trigger-*` headers.
pub(crate) fn verify_attestation_claims(
    claims: &serde_json::Value,
    expected_iss: &str,
    expected_aud: &str,
) -> Result<VerifiedTrigger, BrokerError> {
    let str_claim = |key: &str| claims.get(key).and_then(|v| v.as_str());

    if str_claim("iss") != Some(expected_iss) {
        return Err(BrokerError::Verification("issuer mismatch".into()));
    }
    // `aud` is a string in Connect's attestation; tolerate the array form a
    // general OIDC token may use by checking membership.
    let aud_ok = match claims.get("aud") {
        Some(serde_json::Value::String(s)) => s == expected_aud,
        Some(serde_json::Value::Array(items)) => {
            items.iter().any(|v| v.as_str() == Some(expected_aud))
        }
        _ => false,
    };
    if !aud_ok {
        return Err(BrokerError::Verification("audience mismatch".into()));
    }

    // The anti-decoy check: without this, the receiver's own ambient OIDC token
    // (identical iss/aud/sub) forges a webhook.
    if str_claim("client_id") != Some(CONNECT_CLIENT_ID) {
        return Err(BrokerError::Verification(
            "not a Connect-forwarded attestation (client_id)".into(),
        ));
    }

    let trigger = claims.get("trigger").ok_or_else(|| {
        BrokerError::Verification("attestation carries no `trigger` claim".into())
    })?;
    let trig_str = |key: &str| {
        trigger
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };

    let connector_uid = trig_str("uid")
        .ok_or_else(|| BrokerError::Verification("`trigger` missing `uid`".into()))?;
    let connector_id =
        trig_str("id").ok_or_else(|| BrokerError::Verification("`trigger` missing `id`".into()))?;
    // `service` is the provider; fall back to `type` (they matched in every
    // captured event, but be explicit about the precedence).
    let provider = trig_str("service")
        .or_else(|| trig_str("type"))
        .ok_or_else(|| BrokerError::Verification("`trigger` missing `service`/`type`".into()))?;

    Ok(VerifiedTrigger {
        provider,
        connector_uid,
        connector_id,
    })
}

#[cfg(test)]
mod verify_tests {
    use super::*;

    const ISS: &str = "https://oidc.vercel.com/acme";
    const AUD: &str = "https://vercel.com/acme";

    /// A genuine Connect-forwarded attestation's claims (captured shape).
    fn connect_attestation() -> serde_json::Value {
        serde_json::json!({
            "iss": ISS,
            "aud": AUD,
            "sub": "owner:acme:project:temper-api:environment:production",
            "client_id": "api-connex",
            "trigger": { "id": "scl_abc", "uid": "slack/acme", "type": "slack", "service": "slack" }
        })
    }

    /// The receiving deployment's OWN ambient OIDC token — identical iss/aud/sub,
    /// but no `client_id` and no `trigger`. This is the decoy the anti-decoy check
    /// must reject.
    fn ambient_decoy() -> serde_json::Value {
        serde_json::json!({
            "iss": ISS,
            "aud": AUD,
            "sub": "owner:acme:project:temper-api:environment:production"
        })
    }

    #[test]
    fn accepts_a_genuine_attestation_and_extracts_the_signed_trigger() {
        let t = verify_attestation_claims(&connect_attestation(), ISS, AUD).expect("valid");
        assert_eq!(t.provider, "slack");
        assert_eq!(t.connector_uid, "slack/acme");
        assert_eq!(t.connector_id, "scl_abc");
    }

    #[test]
    fn rejects_the_ambient_token_decoy_despite_matching_iss_aud_sub() {
        let err = verify_attestation_claims(&ambient_decoy(), ISS, AUD).unwrap_err();
        assert!(
            matches!(err, BrokerError::Verification(_)),
            "ambient token must be rejected as a verification failure, got {err:?}"
        );
    }

    #[test]
    fn rejects_a_wrong_audience() {
        let err = verify_attestation_claims(
            &connect_attestation(),
            ISS,
            "https://vercel.com/someone-else",
        )
        .unwrap_err();
        assert!(matches!(err, BrokerError::Verification(_)));
    }

    #[test]
    fn rejects_a_wrong_issuer() {
        let err = verify_attestation_claims(
            &connect_attestation(),
            "https://oidc.vercel.com/someone-else",
            AUD,
        )
        .unwrap_err();
        assert!(matches!(err, BrokerError::Verification(_)));
    }

    #[test]
    fn rejects_a_wrong_client_id_even_with_a_trigger() {
        let mut claims = connect_attestation();
        claims["client_id"] = serde_json::json!("api-something-else");
        let err = verify_attestation_claims(&claims, ISS, AUD).unwrap_err();
        assert!(matches!(err, BrokerError::Verification(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact response body captured from a live Connect GitHub mint
    /// (2026-07-14), token value replaced.
    fn live_github_mint() -> serde_json::Value {
        serde_json::json!({
            "token": "ghs_EXAMPLETOKENVALUE",
            "tokenId": "stk_cjrQxli7X32bhSTKe6pRKw",
            "expiresAt": 1784065923026_i64,
            "connector": { "id": "scl_Fa0hyGE6xDZHJ7DCItpggg", "uid": "github/temper-probe", "type": "github" },
            "installationId": "146616373",
            "metadata": {
                "permissions": { "contents": "write", "metadata": "read", "workflows": "write" },
                "repository_selection": "all"
            }
        })
    }

    #[test]
    fn parses_token_expiry_and_reach_from_a_live_mint() {
        let minted = parse_mint_response(&live_github_mint()).expect("should parse");

        assert_eq!(minted.token.expose(), "ghs_EXAMPLETOKENVALUE");
        // 1784065923026 ms → 2026-07-14T…Z
        assert_eq!(minted.expires_at.timestamp_millis(), 1784065923026);
        // The reach is the metadata object verbatim.
        assert_eq!(
            minted
                .reach
                .raw
                .get("repository_selection")
                .and_then(|v| v.as_str()),
            Some("all")
        );
        assert_eq!(
            minted
                .reach
                .raw
                .pointer("/permissions/contents")
                .and_then(|v| v.as_str()),
            Some("write")
        );
    }
}
