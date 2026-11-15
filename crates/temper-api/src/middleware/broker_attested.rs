//! The `BrokerAttested` tier: the gate on the webhook intake (`routes::webhook_intake`), applied as
//! a layer rather than run by the handler.
//!
//! Two steps, one refusal:
//!
//! 1. **The broker attestation.** The `Authorization` bearer is the attestation, verified by the
//!    configured [`CredentialBroker`](temper_services::broker::CredentialBroker): signature,
//!    issuer, audience, the anti-decoy `client_id`, and the signed `trigger` claim. Not
//!    `x-vercel-oidc-token`, which is this deployment's own ambient identity and is present on
//!    every inbound request (the decoy the `client_id` check exists to refuse).
//! 2. **The connection.** The signed connector identity resolves to exactly one live, credentialed
//!    connection. The broker stops before this deliberately (it stays DB-free, hence swappable).
//!
//! Either failure is the same uniform pre-identity refusal, so a probe learns neither whether a
//! connector is provisioned here, nor how far its attestation got, nor that this door exists. The
//! reason is logged, never returned. Two connections for one connector is a failure (500), not a
//! refusal: temper cannot know which one the payload was for.
//!
//! **Headers only.** The attestation verifies from the bearer, so this layer never reads the body.
//! As a layer it wraps the method router's fallback and every extractor, so nothing on the path
//! (a 405 for an unmounted method, the `Bytes` extractor's 400 for a body it cannot buffer or
//! decompress, how long the door waits for a body, whether it answers `100 Continue`) answers a
//! caller before the gate does. The handler reads the body only after admission, and takes the
//! verified attestation and its connection from [`AttestedConnection`].

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_core::types::connection::Connection;
use temper_services::broker::{InboundRequest, VerifiedInbound};
use temper_services::error::{ApiError, UnauthCause};
use temper_services::services::connection_service;
use temper_services::state::AppState;

/// What the gate admitted: the verified attestation and the connection it names. Planted for the
/// intake handler, which cannot run without it.
#[derive(Clone, Debug)]
pub struct AttestedConnection {
    pub verified: VerifiedInbound,
    pub connection: Connection,
}

fn refusal() -> ApiError {
    ApiError::Unauthenticated(UnauthCause::IntakeUnattested)
}

/// The `BrokerAttested` tier's gate. See the module doc.
pub async fn require_broker_attestation(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let authorization = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let verified = state
        .broker
        .verify_inbound(InboundRequest { authorization })
        .await
        .map_err(|e| {
            // "no Authorization header", "issuer mismatch" and "not a Connect-forwarded
            // attestation" each tell a prober something different about how far it got.
            tracing::warn!(error = %e, "inbound webhook attestation refused");
            refusal()
        })?;

    let connection = connection_service::resolve_inbound(
        &state.pool,
        &verified.provider,
        &verified.connector_uid,
    )
    .await
    .map_err(|e| match e {
        ApiError::Unauthenticated(_) => {
            tracing::warn!(
                provider = %verified.provider,
                "verified attestation names no live credentialed connection"
            );
            refusal()
        }
        other => other,
    })?;

    request.extensions_mut().insert(AttestedConnection {
        verified,
        connection,
    });
    Ok(next.run(request).await)
}
