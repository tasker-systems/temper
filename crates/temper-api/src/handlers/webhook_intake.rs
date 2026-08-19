//! The intake transport — where a real webhook enters temper (S3 of "external systems as
//! subscribed emitters", spec 2026-07-13).
//!
//! `intake_service::receive_webhook` shipped in chunks B and C and was **called by nothing**.
//! This is the caller, and only the caller: it authenticates, resolves, and hands over. The
//! matching and the delivery projection below it are not reopened.
//!
//! ## Auth posture
//!
//! Unauthenticated at the middleware and **self-gated on the broker attestation**, the same
//! posture as `embed_internal_routes` — see `routes::webhook_intake_routes`. There is no
//! `require_auth` because the caller is Vercel Connect forwarding a remote system's event; it
//! holds no temper token and never will. Its compensating control is the RS256/JWKS attestation
//! plus the anti-decoy `client_id` assertion, both inside
//! [`CredentialBroker::verify_inbound`](temper_services::broker::CredentialBroker::verify_inbound).
//!
//! ## What is and is not attested
//!
//! The attestation authenticates the **connector**, not the content. `verify_inbound` returns
//! `payload: req.body.to_vec()` with no claim binding the body, and the live-captured claim set is
//! `iss/aud/sub/kid/client_id/trigger/exp/iat` (research `019f62e6`). So the body and the headers
//! are equally the sender's word, and the event name read from `X-GitHub-Event` is no weaker an
//! input than `repository.full_name` read from the body — both are claims by a party that has
//! proven only *which connector it is*. What the ledger records is therefore the **provenance** of
//! the steering value, not a trust level: see `intake_service::EventTypeSource`.
//!
//! ## Refusal vs. failure, which is the whole of this handler's status mapping
//!
//! - A payload matching **zero** subscriptions is **acked**. The empty radius is the noise filter
//!   (goal C4) and a routed-nowhere payload is a well-formed act the system said no to. Returning
//!   non-2xx would make GitHub retry a payload temper deliberately routed nowhere — and retry it
//!   to the same conclusion, forever.
//! - An unverifiable attestation and an unknown connector return the **same** 401 with the same
//!   body. A probe must not learn whether a connector is provisioned by diffing responses.
//! - Everything temper cannot do — an ambiguous connector, a provider with no event-name rule — is
//!   a **500**, because the system is unable to act rather than saying no. A retry is then the
//!   right behaviour: the condition is a misconfiguration someone will fix.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use temper_services::broker::InboundRequest;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::intake_service::{self, ProviderEvent};
use temper_services::services::{connection_service, intake_service as intake};
use temper_services::state::AppState;

/// The one refusal message for both "the attestation did not verify" and "no connection carries
/// this connector". One constant rather than two equal strings: two strings are two things that
/// can drift, and the moment they differ the pair becomes an existence oracle over which
/// connectors this instance has provisioned.
const REFUSAL: &str = "inbound attestation not accepted";

/// What the sender gets back on a successful receipt. The event id is returned because the
/// caller is the authenticated connector — the party the receipt is *about* — and a delivery id
/// it can correlate is what makes a redelivery diagnosable from the sending side.
#[derive(Debug, Serialize)]
pub struct WebhookAccepted {
    pub event_id: Uuid,
}

/// Receive one webhook.
///
/// The body is taken as raw [`Bytes`], not `Json<Value>`: `verify_inbound` takes `&[u8]` and the
/// payload is stored verbatim, so a deserialize-then-reserialize round trip in between would
/// silently normalize the bytes the ledger claims to have preserved.
pub async fn receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<WebhookAccepted>)> {
    // 1. Authenticate the connector. The `Authorization` bearer is the attestation — NOT
    //    `x-vercel-oidc-token`, which is this deployment's own ambient identity and is present on
    //    every inbound request (the decoy the anti-decoy `client_id` check exists to refuse).
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let verified = state
        .broker
        .verify_inbound(InboundRequest {
            authorization,
            body: &body,
        })
        .await
        .map_err(|e| {
            // The reason is logged, never returned: "no Authorization header", "issuer mismatch"
            // and "not a Connect-forwarded attestation" each tell a prober something different
            // about how far it got.
            tracing::warn!(error = %e, "inbound webhook attestation refused");
            ApiError::Unauthorized(REFUSAL.to_string())
        })?;

    // 2. Resolve the SIGNED connector identity to the connection that receives it. The broker
    //    stops before this deliberately (it stays DB-free, hence swappable), so this is the
    //    caller's job. A no-match is rendered as the same refusal as step 1.
    let connection = connection_service::resolve_inbound(
        &state.pool,
        &verified.provider,
        &verified.connector_uid,
    )
    .await
    .map_err(|e| match e {
        ApiError::Unauthorized(_) => {
            tracing::warn!(
                provider = %verified.provider,
                "verified attestation names no live credentialed connection"
            );
            ApiError::Unauthorized(REFUSAL.to_string())
        }
        other => other,
    })?;

    // 3. Read the provider's own event name. `provider` comes from the signed `trigger` claim, so
    //    the *dispatch* is attested even though the value it selects is not.
    //
    //    A provider with a rule whose header is absent is a FAILURE, not a refusal: the event name
    //    steers the coarse radius, and landing the event without it would compute a radius from
    //    an input temper knows it did not have — reading downstream exactly like a correct empty
    //    radius. Whether Connect forwards a provider's own headers is explicitly unwitnessed
    //    (research `019f62e6`: "whether Connect adds any provider-specific header for GitHub …
    //    assume the same, verify when B5's read-only App exists"), so this is the branch that
    //    makes that assumption fail loudly on the first real forward rather than quietly.
    let event_type = provider_event_type(&verified.provider, &headers)?;

    // 4. Hand over. Everything below here shipped in chunks B and C.
    let event_id = intake::receive_webhook(
        &state.pool,
        connection.id,
        ProviderEvent::from_header(&event_type),
        &parse_payload(&body)?,
    )
    .await?;

    // 202, not 200: temper has accepted and durably recorded the receipt, and what any subscriber
    // makes of it happens later, on a steward's own tick. A payload that matched zero
    // subscriptions arrives here too — that is the noise filter working, not an error.
    Ok((StatusCode::ACCEPTED, Json(WebhookAccepted { event_id })))
}

/// The provider's event name, or the failure that says temper could not read it.
fn provider_event_type(provider: &str, headers: &HeaderMap) -> ApiResult<String> {
    let Some(header_name) = intake_service::event_type_header(provider) else {
        return Err(ApiError::Internal(format!(
            "no event-type rule for provider '{provider}'. temper cannot determine which remote \
             event this is, and the coarse radius is steered by that name — so it must not guess."
        )));
    };
    headers
        .get(header_name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::Internal(format!(
                "provider '{provider}' states its event name in '{header_name}', and the request \
                 carried none. The radius cannot be computed from an input temper does not have."
            ))
        })
}

/// The verbatim body as JSON. `kb_events.payload` is JSONB, so a body that is not JSON cannot be
/// stored verbatim and cannot be a webhook this instance receives. A 400 rather than a 500: the
/// request is malformed, which is the sender's side of the boundary.
fn parse_payload(body: &Bytes) -> ApiResult<serde_json::Value> {
    serde_json::from_slice(body)
        .map_err(|e| ApiError::BadRequest(format!("webhook body is not valid JSON: {e}")))
}
