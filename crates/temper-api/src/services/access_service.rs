// crates/temper-api/src/services/access_service.rs
//! Access gate service — system access checks, join request lifecycle, entitlements.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::access_gate::{
    Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
    SystemSettings,
};
use temper_core::types::ids::EventId;

use crate::error::{ApiError, ApiResult};

// ---------------------------------------------------------------------------
// System access checks (called by middleware)
// ---------------------------------------------------------------------------

/// Check if a profile has system-level access.
/// In `open` mode this always returns true.
/// In `invite_only` mode the profile must be a member of the gating team.
pub async fn has_system_access(pool: &PgPool, profile_id: Uuid) -> ApiResult<bool> {
    let result = sqlx::query_scalar!("SELECT has_system_access($1)", profile_id,)
        .fetch_one(pool)
        .await?;

    Ok(result.unwrap_or(false))
}

/// Check if a profile is a system admin (owner of the gating team).
pub async fn is_system_admin(pool: &PgPool, profile_id: Uuid) -> ApiResult<bool> {
    let result = sqlx::query_scalar!("SELECT is_system_admin($1)", profile_id,)
        .fetch_one(pool)
        .await?;

    Ok(result.unwrap_or(false))
}

// ---------------------------------------------------------------------------
// System settings
// ---------------------------------------------------------------------------

/// Read the singleton system settings row.
pub async fn get_system_settings(pool: &PgPool) -> ApiResult<SystemSettings> {
    let row = sqlx::query_as!(
        SystemSettings,
        "SELECT id, access_mode, gating_team_slug, terms_version, terms_resource_uri, instance_name, updated FROM kb_system_settings LIMIT 1",
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Return the public-safe subset of system settings (no gating_team_slug).
pub async fn get_public_settings(pool: &PgPool) -> ApiResult<PublicSystemSettings> {
    get_system_settings(pool)
        .await
        .map(PublicSystemSettings::from)
}

// ---------------------------------------------------------------------------
// Join request lifecycle
// ---------------------------------------------------------------------------

/// Parameters for creating a join request.
pub struct CreateJoinRequestParams {
    pub profile_id: Uuid,
    pub message: Option<String>,
    pub source: String,
    pub accepted_terms_version: Option<String>,
}

/// Submit a join request for the gating team.
///
/// Returns `BadRequest` if the system is in `open` mode (no request needed).
/// The partial unique index on `kb_join_requests` prevents duplicate pending requests.
pub async fn create_join_request(
    pool: &PgPool,
    params: CreateJoinRequestParams,
) -> ApiResult<JoinRequest> {
    let settings = get_system_settings(pool).await?;

    if settings.access_mode == "open" {
        return Err(ApiError::BadRequest(
            "System is in open mode — no access request needed".to_string(),
        ));
    }

    let gating_slug = settings.gating_team_slug.ok_or_else(|| {
        ApiError::Internal("System is invite_only but no gating team configured".to_string())
    })?;

    // Resolve team ID from slug
    let team_id = sqlx::query_scalar!(
        "SELECT id FROM kb_teams WHERE slug = $1 AND is_active = true",
        gating_slug,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ApiError::Internal(format!("Gating team '{gating_slug}' not found or inactive"))
    })?;

    let request_id = Uuid::now_v7();
    let accepted_terms_at = params
        .accepted_terms_version
        .as_ref()
        .map(|_| chrono::Utc::now());

    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        INSERT INTO kb_join_requests
            (id, team_id, requesting_profile_id, status, message, source,
             accepted_terms_version, accepted_terms_at, created, updated)
        VALUES ($1, $2, $3, 'pending', $4, $5, $6, $7, now(), now())
        RETURNING id, team_id, requesting_profile_id,
                  status as "status: JoinRequestStatus",
                  message, source, accepted_terms_version, accepted_terms_at,
                  reviewed_by_profile_id, reviewed_at, decision_note,
                  created, updated
        "#,
        request_id,
        team_id,
        params.profile_id,
        params.message,
        params.source,
        params.accepted_terms_version,
        accepted_terms_at,
    )
    .fetch_one(pool)
    .await?;

    // Emit audit event
    let payload = JoinRequestEventPayload {
        join_request_id: row.id,
        reviewed_by: None,
        decision_note: None,
    };
    emit_join_request_event(pool, params.profile_id, "join_request.submitted", &payload).await;

    Ok(row)
}

/// Get the most recent join request for this profile against the gating team.
pub async fn get_own_request(pool: &PgPool, profile_id: Uuid) -> ApiResult<Option<JoinRequest>> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Ok(None);
    };

    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        SELECT jr.id, jr.team_id, jr.requesting_profile_id,
               jr.status as "status: JoinRequestStatus",
               jr.message, jr.source, jr.accepted_terms_version, jr.accepted_terms_at,
               jr.reviewed_by_profile_id, jr.reviewed_at, jr.decision_note,
               jr.created, jr.updated
          FROM kb_join_requests jr
          JOIN kb_teams t ON t.id = jr.team_id
         WHERE jr.requesting_profile_id = $1
           AND t.slug = $2
         ORDER BY jr.created DESC
         LIMIT 1
        "#,
        profile_id,
        gating_slug,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Withdraw the pending join request for this profile.
pub async fn withdraw_request(pool: &PgPool, profile_id: Uuid) -> ApiResult<()> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Err(ApiError::NotFound);
    };

    let result = sqlx::query_scalar!(
        r#"
        UPDATE kb_join_requests jr
           SET status = 'withdrawn', updated = now()
          FROM kb_teams t
         WHERE jr.team_id = t.id
           AND jr.requesting_profile_id = $1
           AND t.slug = $2
           AND jr.status = 'pending'
        RETURNING jr.id
        "#,
        profile_id,
        gating_slug,
    )
    .fetch_optional(pool)
    .await?;

    match result {
        Some(request_id) => {
            let payload = JoinRequestEventPayload {
                join_request_id: request_id,
                reviewed_by: None,
                decision_note: None,
            };
            emit_join_request_event(pool, profile_id, "join_request.withdrawn", &payload).await;
            Ok(())
        }
        None => Err(ApiError::NotFound),
    }
}

/// List pending join requests with profile info (admin view).
pub async fn list_pending_requests(pool: &PgPool) -> ApiResult<Vec<JoinRequestWithProfile>> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Ok(vec![]);
    };

    let rows = sqlx::query_as!(
        JoinRequestWithProfile,
        r#"
        SELECT jr.id, jr.team_id, jr.requesting_profile_id,
               jr.status as "status: JoinRequestStatus",
               jr.message, jr.source, jr.accepted_terms_version, jr.accepted_terms_at,
               jr.reviewed_by_profile_id, jr.reviewed_at, jr.decision_note,
               jr.created, jr.updated,
               p.display_name, p.email
          FROM kb_join_requests jr
          JOIN kb_teams t ON t.id = jr.team_id
          JOIN kb_profiles p ON p.id = jr.requesting_profile_id
         WHERE t.slug = $1
           AND jr.status = 'pending'
         ORDER BY jr.created DESC
        "#,
        gating_slug,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Parameters for reviewing (approving/rejecting) a join request.
pub struct ReviewRequestParams {
    pub request_id: Uuid,
    pub reviewer_profile_id: Uuid,
    pub decision: JoinRequestStatus,
    pub decision_note: Option<String>,
}

/// Approve or reject a join request. On approval, atomically insert team membership.
pub async fn review_request(pool: &PgPool, params: ReviewRequestParams) -> ApiResult<JoinRequest> {
    if params.decision != JoinRequestStatus::Approved
        && params.decision != JoinRequestStatus::Rejected
    {
        return Err(ApiError::BadRequest(
            "Decision must be 'approved' or 'rejected'".to_string(),
        ));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    // Update the join request
    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        UPDATE kb_join_requests
           SET status = $2,
               reviewed_by_profile_id = $3,
               reviewed_at = now(),
               decision_note = $4,
               updated = now()
         WHERE id = $1
           AND status = 'pending'
        RETURNING id, team_id, requesting_profile_id,
                  status as "status: JoinRequestStatus",
                  message, source, accepted_terms_version, accepted_terms_at,
                  reviewed_by_profile_id, reviewed_at, decision_note,
                  created, updated
        "#,
        params.request_id,
        params.decision as JoinRequestStatus,
        params.reviewer_profile_id,
        params.decision_note,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    // On approval, insert team membership
    if params.decision == JoinRequestStatus::Approved {
        let member_id = Uuid::now_v7();
        sqlx::query!(
            r#"
            INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at, invited_by_profile_id)
            VALUES ($1, $2, $3, 'watcher', now(), $4)
            ON CONFLICT (team_id, profile_id) DO NOTHING
            "#,
            member_id,
            row.team_id,
            row.requesting_profile_id,
            params.reviewer_profile_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    // Emit audit event (outside transaction — best-effort)
    let (event_type, payload) = if params.decision == JoinRequestStatus::Approved {
        (
            "join_request.approved",
            JoinRequestEventPayload {
                join_request_id: row.id,
                reviewed_by: Some(params.reviewer_profile_id),
                decision_note: None,
            },
        )
    } else {
        (
            "join_request.rejected",
            JoinRequestEventPayload {
                join_request_id: row.id,
                reviewed_by: Some(params.reviewer_profile_id),
                decision_note: params.decision_note,
            },
        )
    };
    emit_join_request_event(pool, row.requesting_profile_id, event_type, &payload).await;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Entitlements
// ---------------------------------------------------------------------------

/// Build the entitlements object for a profile.
pub async fn get_entitlements(pool: &PgPool, profile_id: Uuid) -> ApiResult<Entitlements> {
    let system_access = has_system_access(pool, profile_id).await?;
    let is_admin = is_system_admin(pool, profile_id).await?;
    let request = get_own_request(pool, profile_id).await?;

    Ok(Entitlements {
        system_access,
        is_admin,
        join_request_status: request.map(|r| r.status),
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Typed payload for join request lifecycle audit events.
#[derive(serde::Serialize)]
struct JoinRequestEventPayload {
    join_request_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    reviewed_by: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_note: Option<String>,
}

/// Emit a join request lifecycle event to kb_events (best-effort, no error propagation).
async fn emit_join_request_event(
    pool: &PgPool,
    profile_id: Uuid,
    event_type: &str,
    payload: &JoinRequestEventPayload,
) {
    let event_id = EventId::new();
    let payload_json = serde_json::to_value(payload)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));

    let _ = sqlx::query!(
        "INSERT INTO kb_events (id, profile_id, device_id, event_type_id, payload, created)
         VALUES ($1, $2, 'system', resolve_event_type($3), $4, now())",
        event_id as EventId,
        profile_id,
        event_type,
        payload_json,
    )
    .execute(pool)
    .await;
}
