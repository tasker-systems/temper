// crates/temper-api/src/services/access_service.rs
//! Access gate service — system access checks, join request lifecycle, entitlements.
//!
//! Admin/operational events are firewalled from the cognition ledger
//! (`kb_events`): the substrate `kb_events` is cognition-only (entity emitters,
//! context/cogmap anchors), so the join-request lifecycle is NOT ledgered there.
//! The audit trail lives on `kb_join_requests` (status / reviewed_by_profile_id /
//! timestamps) plus the `kb_team_members` row created on approval. A dedicated
//! admin-event sink is a future deliverable.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::access_gate::{
    AccessMode, Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile,
    PublicSystemSettings, SystemSettings,
};
use temper_core::types::admin::UpdateSettingsRequest;
use temper_core::types::cognitive_maps::{
    GrantCapabilityRequest, GrantOutcome, RevokeCapabilityRequest, RevokeOutcome,
};
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::team::{TeamMemberRow, TeamRole};

use crate::error::{ApiError, ApiResult};

// ---------------------------------------------------------------------------
// System access checks (called by middleware)
// ---------------------------------------------------------------------------

/// Check if a profile has system-level access.
/// In `open` mode this always returns true.
/// In `invite_only` mode the profile must be a member of the gating team.
pub async fn has_system_access(pool: &PgPool, profile_id: ProfileId) -> ApiResult<bool> {
    let result = sqlx::query_scalar!("SELECT has_system_access($1)", *profile_id,)
        .fetch_one(pool)
        .await?;

    Ok(result.unwrap_or(false))
}

/// Check if a profile is a system admin (owner of the gating team).
pub async fn is_system_admin(pool: &PgPool, profile_id: ProfileId) -> ApiResult<bool> {
    let result = sqlx::query_scalar!("SELECT is_system_admin($1)", *profile_id,)
        .fetch_one(pool)
        .await?;

    Ok(result.unwrap_or(false))
}

// ---------------------------------------------------------------------------
// Access-capability grants (D3b §3.C) — the surface-facing writers of
// `kb_access_grants`. Admin events (firewalled from cognition, memory
// project_admin_eventsourcing_and_operating_shape): called DIRECTLY from
// surfaces, like `cogmap_service::bind_team`, NOT via the DbBackend trait.
// ---------------------------------------------------------------------------

/// Grant-administration gate: a system admin OR a holder of `can_grant` on the subject (the general
/// `can(...,'grant',...)` seam). This is a DIFFERENT axis from authoring — authoring stays wholly
/// explicit (D3b §3.E), while grant-administration admits admins so pre-existing maps (no seeded
/// `can_grant` holder) and repair stay operable.
///
/// `pub(crate)` for `admin_ledger_service`, whose READ gate mirrors this WRITE gate by CALLING it
/// (spec 2026-07-16 §5): if you could perform the act, you may read the record of it. Restating the
/// predicate there would be a second copy of the policy that drifts from the gate it exists to
/// mirror — tighten this fn and the ledger's read gate tightens with it.
pub(crate) async fn can_administer_grant(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<bool> {
    Ok(is_system_admin(pool, caller).await?
        || profile_can_grant(pool, caller, subject_table, subject_id).await?)
}

/// Raw `can_grant` capability probe (NO `is_system_admin` OR) — the reusable primitive. Callers that
/// also admit admins compose it with `is_system_admin` themselves (see `can_administer_grant`,
/// `cogmap_service::can_bind`).
pub(crate) async fn profile_can_grant(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<bool> {
    let ok = sqlx::query_scalar!(
        "SELECT can('kb_profiles', $1, 'grant', $2, $3)",
        *caller,
        subject_table,
        subject_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}

/// Is `team_id` the configured gating/root team? An unconfigured system (`gating_team_slug` NULL)
/// has no gating team ⇒ `false`. Used by the bind gate's escalation guard: binding a map to the
/// gating team flips it into the `require_cogmap_write_admin` regime, so it stays admin-only.
pub(crate) async fn is_gating_team(pool: &PgPool, team_id: Uuid) -> ApiResult<bool> {
    let ok = sqlx::query_scalar!(
        "SELECT EXISTS( \
           SELECT 1 FROM kb_teams t \
             JOIN kb_system_settings s ON t.slug = s.gating_team_slug \
            WHERE t.id = $1 )",
        team_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}

/// The columns of one `kb_access_grants` upsert. A params struct because the
/// insert takes seven domain values (repo rule: >5 ⇒ struct).
#[derive(Debug, Clone)]
pub struct InsertGrantParams {
    pub subject_table: String,
    pub subject_id: Uuid,
    pub principal_table: String,
    pub principal_id: Uuid,
    pub can_read: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub can_grant: bool,
    pub granted_by_profile_id: Uuid,
}

/// Raw upsert of one access grant, on a connection so it can join a transaction.
/// **Performs no authorization** — every caller must gate first (auth before writes).
/// Returns whether the row was freshly inserted (`xmax = 0`) rather than updated.
pub async fn insert_grant(conn: &mut sqlx::PgConnection, p: &InsertGrantParams) -> ApiResult<bool> {
    // `xmax = 0` distinguishes a fresh INSERT from an ON CONFLICT UPDATE (xmax is the deleting/locking
    // txid — zero only on a row this txn just inserted).
    let inserted = sqlx::query_scalar!(
        r#"INSERT INTO kb_access_grants
               (subject_table, subject_id, principal_table, principal_id,
                can_read, can_write, can_delete, can_grant, granted_by_profile_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
           DO UPDATE SET can_read = EXCLUDED.can_read, can_write = EXCLUDED.can_write,
                         can_delete = EXCLUDED.can_delete, can_grant = EXCLUDED.can_grant,
                         granted_by_profile_id = EXCLUDED.granted_by_profile_id, granted_at = now()
           RETURNING (xmax = 0) AS "inserted!""#,
        p.subject_table,
        p.subject_id,
        p.principal_table,
        p.principal_id,
        p.can_read,
        p.can_write,
        p.can_delete,
        p.can_grant,
        p.granted_by_profile_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(inserted)
}

/// Raw delete of one access grant by its `(subject, principal)` 4-tuple, on a connection so it can
/// join a transaction. **Performs no authorization** — every caller must gate first (auth before
/// writes). Returns whether a row was removed (absent ⇒ `false`, idempotent no-op).
pub async fn delete_grant(
    conn: &mut sqlx::PgConnection,
    subject_table: &str,
    subject_id: Uuid,
    principal_table: &str,
    principal_id: Uuid,
) -> ApiResult<bool> {
    let deleted = sqlx::query!(
        r#"DELETE FROM kb_access_grants
            WHERE subject_table = $1 AND subject_id = $2
              AND principal_table = $3 AND principal_id = $4"#,
        subject_table,
        subject_id,
        principal_table,
        principal_id,
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(deleted > 0)
}

/// Mint/update one access grant. Auth before write: `can_administer_grant`. The DB coherence CHECK
/// (`write|delete|grant ⇒ read`) is the integrity backstop. Idempotent upsert — `granted=false` when
/// the row already existed and was updated in place.
pub async fn grant_capability(
    pool: &PgPool,
    caller: ProfileId,
    req: &GrantCapabilityRequest,
) -> ApiResult<GrantOutcome> {
    if !can_administer_grant(pool, caller, &req.subject_table, req.subject_id).await? {
        return Err(ApiError::Forbidden);
    }
    let mut conn = pool.acquire().await?;
    let granted = insert_grant(
        &mut conn,
        &InsertGrantParams {
            subject_table: req.subject_table.clone(),
            subject_id: req.subject_id,
            principal_table: req.principal_table.clone(),
            principal_id: req.principal_id,
            can_read: req.can_read,
            can_write: req.can_write,
            can_delete: req.can_delete,
            can_grant: req.can_grant,
            granted_by_profile_id: *caller,
        },
    )
    .await?;
    Ok(GrantOutcome { granted })
}

/// Delete one access grant. Auth before write: `can_administer_grant`. Absent row ⇒ no-op success
/// (idempotent, mirrors `bind_team`/`unbind_team`).
pub async fn revoke_capability(
    pool: &PgPool,
    caller: ProfileId,
    req: &RevokeCapabilityRequest,
) -> ApiResult<RevokeOutcome> {
    if !can_administer_grant(pool, caller, &req.subject_table, req.subject_id).await? {
        return Err(ApiError::Forbidden);
    }
    let mut conn = pool.acquire().await?;
    let revoked = delete_grant(
        &mut conn,
        &req.subject_table,
        req.subject_id,
        &req.principal_table,
        req.principal_id,
    )
    .await?;
    Ok(RevokeOutcome { revoked })
}

/// The reserved L0 kernel cognitive map (`20260625000001_l0_kernel_cogmap.sql`). Its write gate is
/// fail-CLOSED and independent of `gating_team_slug`: the kernel is immutable until an operator
/// intentionally configures gating + promotes an admin. See [`require_cogmap_write_admin`].
const L0_KERNEL_COGMAP: CogmapId =
    CogmapId(Uuid::from_u128(0x00000000_0000_0000_0005_000000000001));

/// Structural write-gate. A write requires `is_system_admin` when EITHER:
/// - the target is the reserved **L0 kernel** map (unconditionally — independent of
///   `gating_team_slug`), OR
/// - the target cogmap is joined to the gating (root) team.
///
/// Otherwise the write is ungated here (returns `Ok`) — its own access rules apply elsewhere.
///
/// The L0 special-case is **fail-CLOSED**: when gating is unconfigured (`gating_team_slug` NULL, the
/// canonical-seed default), the root-join EXISTS finds nothing AND `is_system_admin` is false for
/// everyone — so L0 is immutable (denied to all) until an operator configures gating. Without the
/// unconditional L0 branch the gate would be fail-OPEN (any authed user could rewrite the kernel out
/// of the box), because a NULL `gating_team_slug` makes the root-join branch return `Ok` for everyone.
pub async fn require_cogmap_write_admin(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: CogmapId,
) -> ApiResult<()> {
    let is_reserved = cogmap_id == L0_KERNEL_COGMAP;

    let is_root_joined: bool = sqlx::query_scalar!(
        "SELECT EXISTS( \
           SELECT 1 FROM kb_team_cogmaps tc \
             JOIN kb_teams t ON t.id = tc.team_id \
             JOIN kb_system_settings s ON t.slug = s.gating_team_slug \
            WHERE tc.cogmap_id = $1 )",
        *cogmap_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !is_reserved && !is_root_joined {
        return Ok(()); // gate doesn't apply to non-reserved, non-root-team cogmaps
    }
    if is_system_admin(pool, profile_id).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
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

/// Admin-only partial update of the singleton `kb_system_settings` row.
///
/// COALESCE semantics: each `Some` field overwrites its column; each `None`
/// leaves the column unchanged. `access_mode` is validated against
/// `{open, invite_only}`. Guards against the lockout footgun: an effective
/// `invite_only` mode with no `gating_team_slug` would make `has_system_access`
/// false for everyone, so it is rejected.
pub async fn update_system_settings(
    pool: &PgPool,
    req: &UpdateSettingsRequest,
) -> ApiResult<SystemSettings> {
    // Validate access_mode (parse-don't-validate against the DB CHECK).
    if let Some(mode) = req.access_mode.as_deref() {
        if AccessMode::from_db_str(mode).is_none() {
            return Err(ApiError::BadRequest(format!(
                "invalid access_mode {mode:?} (expected 'open' or 'invite_only')"
            )));
        }
    }

    // Compute the EFFECTIVE post-update mode + gating slug to guard lockout.
    let current = get_system_settings(pool).await?;
    let effective_mode = req
        .access_mode
        .clone()
        .unwrap_or(current.access_mode.clone());
    let effective_gating = req
        .gating_team_slug
        .clone()
        .or(current.gating_team_slug.clone());
    if effective_mode == "invite_only" && effective_gating.is_none() {
        return Err(ApiError::BadRequest(
            "invite_only mode requires a gating_team_slug (set --gating-team in the same call \
             or beforehand) — otherwise no one can access the instance"
                .to_string(),
        ));
    }
    // Guard: if the effective gating slug names a team that doesn't exist,
    // enabling invite_only would lock everyone out.
    if effective_mode == "invite_only" {
        if let Some(ref slug) = effective_gating {
            let exists: bool = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM kb_teams WHERE slug = $1)",
                slug
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if !exists {
                return Err(ApiError::BadRequest(format!(
                    "gating_team_slug '{slug}' does not exist — create the team before enabling invite_only"
                )));
            }
        }
    }

    let row = sqlx::query_as!(
        SystemSettings,
        r#"
        UPDATE kb_system_settings
           SET access_mode        = COALESCE($1, access_mode),
               gating_team_slug   = COALESCE($2, gating_team_slug),
               instance_name      = COALESCE($3, instance_name),
               terms_version      = COALESCE($4, terms_version),
               terms_resource_uri = COALESCE($5, terms_resource_uri),
               updated            = now()
         WHERE id = 1
        RETURNING id, access_mode, gating_team_slug, terms_version,
                  terms_resource_uri, instance_name, updated
        "#,
        req.access_mode,
        req.gating_team_slug,
        req.instance_name,
        req.terms_version,
        req.terms_resource_uri,
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Admin-only: grant `profile_id` the `owner` role on a team (idempotent).
///
/// `team_id == None` resolves to the configured gating team — system-admin ≡
/// owner of the gating team, so this mints a second system admin. Decoupled
/// from `kb_profiles.system_access` (the auth gate reads gating-team ownership,
/// not the enum). Auth is enforced by the caller (handler `is_system_admin`).
pub async fn promote_admin(
    pool: &PgPool,
    profile_id: Uuid,
    team_id: Option<Uuid>,
) -> ApiResult<TeamMemberRow> {
    // Resolve the target team: explicit, else the configured gating team.
    let target_team = match team_id {
        Some(id) => {
            let exists: bool =
                sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1)", id)
                    .fetch_one(pool)
                    .await?
                    .unwrap_or(false);
            if !exists {
                return Err(ApiError::BadRequest(format!("team '{id}' does not exist")));
            }
            id
        }
        None => {
            let settings = get_system_settings(pool).await?;
            let Some(slug) = settings.gating_team_slug else {
                return Err(ApiError::BadRequest(
                    "no gating team configured; pass --team to promote on a specific team"
                        .to_string(),
                ));
            };
            sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", slug)
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("gating team '{slug}' does not exist"))
                })?
        }
    };

    // Validate the target profile exists before writing.
    let profile_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE id = $1)",
        profile_id
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !profile_exists {
        return Err(ApiError::BadRequest(format!(
            "profile '{profile_id}' does not exist"
        )));
    }

    let row = sqlx::query_as!(
        TeamMemberRow,
        r#"
        INSERT INTO kb_team_members (team_id, profile_id, role)
        VALUES ($1, $2, 'owner')
        ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role
        RETURNING team_id, profile_id, role AS "role: TeamRole", created
        "#,
        target_team,
        profile_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Join request lifecycle
// ---------------------------------------------------------------------------

/// Parameters for creating a join request.
pub struct CreateJoinRequestParams {
    pub profile_id: ProfileId,
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

    let access_mode = AccessMode::from_db_str(&settings.access_mode).ok_or_else(|| {
        ApiError::Internal(format!(
            "unrecognized access_mode {:?} in kb_system_settings",
            settings.access_mode
        ))
    })?;
    match access_mode {
        AccessMode::Open => {
            return Err(ApiError::BadRequest(
                "System is in open mode — no access request needed".to_string(),
            ));
        }
        AccessMode::InviteOnly => {}
    }

    let gating_slug = settings.gating_team_slug.ok_or_else(|| {
        ApiError::Internal("System is invite_only but no gating team configured".to_string())
    })?;

    // Resolve team ID from slug (substrate `kb_teams` has no `is_active`).
    let team_id = sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", gating_slug,)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("Gating team '{gating_slug}' not found")))?;

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
        *params.profile_id,
        params.message,
        params.source,
        params.accepted_terms_version,
        accepted_terms_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Get the most recent join request for this profile against the gating team.
pub async fn get_own_request(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Option<JoinRequest>> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Ok(None);
    };

    // `vw_join_requests` (migration 20260709000003) carries the one shared projection +
    // team/profile joins; view columns infer nullable, so the non-null columns take `!`
    // overrides matching the JoinRequest shape.
    let row = sqlx::query_as!(
        JoinRequest,
        r#"
        SELECT id as "id!", team_id as "team_id!",
               requesting_profile_id as "requesting_profile_id!",
               status as "status!: JoinRequestStatus",
               message, source as "source!", accepted_terms_version, accepted_terms_at,
               reviewed_by_profile_id, reviewed_at, decision_note,
               created as "created!", updated as "updated!"
          FROM vw_join_requests
         WHERE requesting_profile_id = $1
           AND team_slug = $2
         ORDER BY created DESC
         LIMIT 1
        "#,
        *profile_id,
        gating_slug,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Withdraw the pending join request for this profile.
pub async fn withdraw_request(pool: &PgPool, profile_id: ProfileId) -> ApiResult<()> {
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
        *profile_id,
        gating_slug,
    )
    .fetch_optional(pool)
    .await?;

    match result {
        Some(_request_id) => Ok(()),
        None => Err(ApiError::NotFound),
    }
}

/// List pending join requests with profile info (admin view).
pub async fn list_pending_requests(pool: &PgPool) -> ApiResult<Vec<JoinRequestWithProfile>> {
    let settings = get_system_settings(pool).await?;

    let Some(gating_slug) = settings.gating_team_slug else {
        return Ok(vec![]);
    };

    // Same `vw_join_requests` projection as `get_own_request`, plus the view's joined
    // requester columns (`display_name`/`email`) for the admin queue shape.
    let rows = sqlx::query_as!(
        JoinRequestWithProfile,
        r#"
        SELECT id as "id!", team_id as "team_id!",
               requesting_profile_id as "requesting_profile_id!",
               status as "status!: JoinRequestStatus",
               message, source as "source!", accepted_terms_version, accepted_terms_at,
               reviewed_by_profile_id, reviewed_at, decision_note,
               created as "created!", updated as "updated!",
               display_name as "display_name!", email
          FROM vw_join_requests
         WHERE team_slug = $1
           AND status = 'pending'
         ORDER BY created DESC
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
    pub reviewer_profile_id: ProfileId,
    pub decision: JoinRequestStatus,
    pub decision_note: Option<String>,
}

/// Approve or reject a join request. On approval, atomically insert the
/// substrate-shaped team membership row (no `id`/`joined_at`/`invited_by_profile_id`;
/// reviewer attribution survives on `kb_join_requests.reviewed_by_profile_id`).
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
        *params.reviewer_profile_id,
        params.decision_note,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    // On approval, insert substrate-shaped team membership.
    if params.decision == JoinRequestStatus::Approved {
        sqlx::query!(
            r#"
            INSERT INTO kb_team_members (team_id, profile_id, role)
            VALUES ($1, $2, 'watcher')
            ON CONFLICT (team_id, profile_id) DO NOTHING
            "#,
            row.team_id,
            row.requesting_profile_id,
        )
        .execute(&mut *tx)
        .await?;

        // Now that gating-team membership establishes access, enroll the
        // profile into the rest of the auto-join "everyone" pool (Chunk 1's
        // deferred call site). No-op if has_system_access is still false.
        sqlx::query!(
            "SELECT ensure_auto_join_memberships($1)",
            row.requesting_profile_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Entitlements
// ---------------------------------------------------------------------------

/// Build the entitlements object for a profile.
pub async fn get_entitlements(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Entitlements> {
    let system_access = has_system_access(pool, profile_id).await?;
    let is_admin = is_system_admin(pool, profile_id).await?;
    let request = get_own_request(pool, profile_id).await?;

    Ok(Entitlements {
        system_access,
        is_admin,
        join_request_status: request.map(|r| r.status),
    })
}
