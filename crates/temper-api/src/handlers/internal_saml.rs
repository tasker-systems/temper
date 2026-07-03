//! Internal SAML membership-reconcile endpoint. Called server-to-server by the co-deployed
//! Authorization Server after it validates an assertion, BEFORE it mints the token. Gated by
//! `require_internal_signature` (HMAC over the body, not JWT). See the Phase 2 design spec §7.2
//! and docs/auth/reconcile-channel.md.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use temper_core::types::{AuthClaims, ReconcileRequest};
use temper_services::error::ApiError;
use temper_services::services::{profile_service, saml_provisioning_service};
use temper_services::state::AppState;

/// `POST /internal/saml/reconcile` — resolve/JIT the profile, then reconcile its idp memberships.
pub async fn reconcile(
    State(state): State<AppState>,
    Json(req): Json<ReconcileRequest>,
) -> Result<StatusCode, ApiError> {
    // Identity provider string is authoritative from server config, NOT the payload — this MUST
    // match middleware/auth.rs so the resolved profile is the same one the minted token resolves to.
    let claims = AuthClaims {
        principal_kind: temper_core::types::PrincipalKind::Human,
        provider: state.config.auth_provider_name.clone(),
        external_user_id: req.external_user_id.clone(),
        email: req.email.clone(),
        email_verified: req.email_verified,
        // exp/iat are unused by resolve_from_claims; supply zero rather than inventing a clock.
        exp: 0,
        iat: 0,
    };

    let profile = profile_service::resolve_from_claims(&state.pool, &claims).await?;

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
