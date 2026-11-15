//! The intake transport — where a real webhook enters temper (S3 of "external systems as
//! subscribed emitters", spec 2026-07-13).
//!
//! `intake_service::receive_webhook` shipped in chunks B and C and was **called by nothing**.
//! This is the caller, and only the caller: it authenticates, resolves, and hands over. The
//! matching and the delivery projection below it are not reopened.
//!
//! ## Auth posture
//!
//! Gated by the **`BrokerAttested` tier** (`middleware::broker_attested`), a layer that verifies the
//! broker attestation from its headers and resolves the connection before routing detail or the
//! body runs. There is no `require_auth` because the caller is Vercel Connect forwarding a remote
//! system's event; it holds no temper token and never will. Its control is the RS256/JWKS
//! attestation plus the anti-decoy `client_id` assertion, both inside
//! [`CredentialBroker::verify_inbound`](temper_services::broker::CredentialBroker::verify_inbound).
//!
//! ## What the attestation establishes
//!
//! The attestation authenticates the **connector**: its claims
//! (`iss/aud/sub/client_id/trigger/project_id/exp/iat`, header `kid`; research `019f62e6`) name the
//! connector it was issued for and the project it was issued to. For the event name, read from
//! `X-GitHub-Event`, the ledger records which provider convention supplied it (its
//! **provenance**), not a trust level: see `intake_service::EventTypeSource`.
//!
//! ## Refusal vs. failure, which is the whole of this handler's status mapping
//!
//! - A payload matching **zero** subscriptions is **acked**. The empty radius is the noise filter
//!   (goal C4) and a routed-nowhere payload is a well-formed act the system said no to. Returning
//!   non-2xx would make GitHub retry a payload temper deliberately routed nowhere — and retry it
//!   to the same conclusion, forever.
//! - An unverifiable attestation, an unknown connector, and — once the binding is enforced — an
//!   attestation issued to another project or presented again after its burst window all return
//!   the **same** 401 with the same body. A probe must not learn whether a connector is provisioned, or how far its
//!   attestation got, by diffing responses.
//! - Everything temper cannot do — an ambiguous connector, a provider with no event-name rule — is
//!   a **500**, because the system is unable to act rather than saying no. A retry is then the
//!   right behaviour: the condition is a misconfiguration someone will fix.
//!
//! ## GitHub's own signature, observed
//!
//! Each verified receipt for a provisioned GitHub connection logs one `hub_signature` field:
//! whether the forwarded `X-Hub-Signature-256` arrived, and, when `GITHUB_WEBHOOK_SECRET` is
//! configured, whether it verifies over the exact bytes received. Logging only; the receipt is
//! handled the same whichever value it logs. See `intake_service::HubSignature`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use serde::Serialize;
use uuid::Uuid;

use temper_services::error::{ApiError, ApiResult, UnauthCause};
use temper_services::services::intake_service as intake;
use temper_services::services::intake_service::{self, AttestationSeen, ProviderEvent, Receipt};
use temper_services::state::AppState;

use crate::middleware::broker_attested::AttestedConnection;

/// The one refusal for every way an inbound webhook is turned away before it is accepted: the
/// attestation did not verify, it names no live connection, or it was replayed. The uniform
/// pre-identity refusal, so a probe learns neither whether a connector is provisioned, nor how far
/// its attestation got, nor that this door exists.
fn refusal() -> ApiError {
    ApiError::Unauthenticated(UnauthCause::IntakeUnattested)
}

/// What the sender gets back on a successful receipt. The event id is returned because the
/// caller is the authenticated connector — the party the receipt is *about* — and a delivery id
/// it can correlate is what makes a redelivery diagnosable from the sending side.
#[derive(Debug, Serialize)]
pub struct WebhookAccepted {
    pub event_id: Uuid,
}

/// Receive one webhook, already admitted by the `BrokerAttested` tier
/// (`middleware::broker_attested`), which verified the attestation from its headers and resolved
/// the connection before anything read the body.
///
/// The body is taken as raw [`Bytes`], not `Json<Value>`: the payload is stored verbatim, so a
/// deserialize-then-reserialize round trip would silently normalize the bytes the ledger claims to
/// have preserved.
pub async fn receive(
    State(state): State<AppState>,
    Extension(attested): Extension<AttestedConnection>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<WebhookAccepted>)> {
    let AttestedConnection {
        verified,
        connection,
    } = attested;

    // A precondition header is refused by the group's precondition layer, inside the tier gate:
    // a caller the gate refuses gets its 401 exactly as without the header, and an admitted one is
    // refused 400 before anything below runs. One refusal comes later and so is answered 400 when
    // a header is sent: a replayed attestation (step 4), which can only be presented by a caller
    // that already held a valid one for a provisioned connection.

    // Observe GitHub's own signature on a receipt for a provisioned GitHub connection, before any
    // later step can refuse or fail it, so each such receipt produces one line. Logging only.
    if verified.provider == "github" {
        let hub_signature = intake_service::classify_hub_signature(
            state.config.github_webhook_secret.as_deref(),
            headers
                .get(intake_service::GITHUB_SIGNATURE_HEADER)
                .map(|v| v.as_bytes()),
            &body,
        );
        tracing::info!(
            provider = %verified.provider,
            hub_signature = hub_signature.as_str(),
            "github webhook signature"
        );
    }

    // Read the provider's own event name. `provider` comes from the signed `trigger` claim, so
    //    the dispatch is attested.
    //
    //    A provider with a rule whose header is absent is a FAILURE, not a refusal: the event name
    //    steers the coarse radius, and landing the event without it would compute a radius from
    //    an input temper knows it did not have — reading downstream exactly like a correct empty
    //    radius. Whether Connect forwards a provider's own headers is explicitly unwitnessed
    //    (research `019f62e6`: "whether Connect adds any provider-specific header for GitHub …
    //    assume the same, verify when B5's read-only App exists"), so this is the branch that
    //    makes that assumption fail loudly on the first real forward rather than quietly.
    let event_type = provider_event_type(&verified.provider, &headers)?;

    // Hand over, recording the attestation in the same transaction as the event: it is
    //    recorded exactly when a receipt lands, and a receipt that fails or is cut off never
    //    records it. Connect sends one attestation for every delivery in a burst, so a repeat
    //    within `ATTESTATION_REPEAT_WINDOW_SECS` of the first lands; a later one is a replay.
    //    Whether a replay is refused or only reported is the broker's binding flag; refused, it
    //    is the same 401 as every other refusal here.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let receipt = intake::receive_attested_webhook(
        &state.pool,
        connection.id,
        ProviderEvent::from_header(&event_type),
        &parse_payload(&body)?,
        AttestationSeen {
            replay_key: &verified.replay_key,
            expires_at: verified.expires_at,
            now,
            enforced: verified.binding_enforced,
        },
    )
    .await?;
    let event_id = match receipt {
        Receipt::Recorded(event_id) => event_id,
        Receipt::RefusedAsReplay => return Err(refusal()),
    };

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
