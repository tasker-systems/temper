//! Internal SAML membership-reconcile endpoint. Called server-to-server by the co-deployed
//! Authorization Server after it validates an assertion, BEFORE it mints the token. Gated by
//! `require_internal_signature` (HMAC over the body, not JWT). See the Phase 2 design spec §7.2
//! and docs/auth/reconcile-channel.md.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use temper_core::types::{ReconcileRequest, ResolvePrincipalRequest, ResolvePrincipalResponse};
use temper_services::error::ApiError;
use temper_services::services::saml_provisioning_service;
use temper_services::state::AppState;

/// `POST /internal/saml/reconcile` — resolve/JIT the profile, then reconcile its idp memberships.
pub async fn reconcile(
    State(state): State<AppState>,
    Json(req): Json<ReconcileRequest>,
) -> Result<StatusCode, ApiError> {
    // The federated seam owns the claims: this was the third site hand-building a
    // `PrincipalKind::Human`, and a surface that can construct one can forge one.
    // The identity provider string is authoritative from server config, NOT the payload —
    // the seam is handed `auth_provider_name` so the profile it resolves is the same one the
    // token the AS is about to mint will resolve to through `authenticate_token`.
    let profile = temper_services::auth::resolve_federated_human(
        &state.pool,
        &state.config.auth_provider_name,
        &req.external_user_id,
        &req.email,
        req.email_verified,
    )
    .await?;

    let outcome = saml_provisioning_service::reconcile_idp_memberships(
        &state.pool,
        profile.id,
        &req.idp_key,
        &req.groups,
    )
    .await?;

    tracing::info!(
        profile_id = %profile.id,
        idp_key = %req.idp_key,
        added = outcome.added,
        updated = outcome.updated,
        revoked = outcome.revoked,
        skipped_native = outcome.skipped_native,
        "saml reconcile complete",
    );

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /internal/principal/resolve` — resolve (or JIT) the profile a token `sub` names.
///
/// The AS calls this so it can stamp a refresh-token row with the principal that owns it, which is
/// what lets `standing_service::apply` end that principal's live chains when their admission ends.
/// It resolves through the SAME seam and the SAME server-configured provider as
/// [`reconcile`] — and as `authenticate_token` — so the owner recorded on the chain is exactly the
/// profile the tokens minted from it will later resolve to. A second copy of that provider name in
/// the AS's own environment is the thing this endpoint exists to avoid.
///
/// Creates nothing the login was not already going to create: `reconcile` JITs the same profile
/// moments later on any assertion carrying groups, and `authenticate_token` JITs it on the first
/// API call otherwise.
pub async fn resolve_principal(
    State(state): State<AppState>,
    Json(req): Json<ResolvePrincipalRequest>,
) -> Result<Json<ResolvePrincipalResponse>, ApiError> {
    let profile = temper_services::auth::resolve_federated_human(
        &state.pool,
        &state.config.auth_provider_name,
        &req.external_user_id,
        &req.email,
        req.email_verified,
    )
    .await?;

    Ok(Json(ResolvePrincipalResponse {
        profile_id: profile.id,
    }))
}
