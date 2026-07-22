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
use temper_substrate::ids::EntityId;
use uuid::Uuid;

use crate::auth::SystemAdmin;
use crate::services::standing_service::{self, ApplyStandingParams};
use temper_principal::{Act, ActorAuthority};

use temper_core::types::access_gate::{
    Entitlements, JoinRequest, JoinRequestStatus, JoinRequestWithProfile, PublicSystemSettings,
    SystemSettings,
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

/// Why a caller may administer grants on a subject — not merely *whether*. Callers need the
/// distinction: attenuation (5b.3) binds a **delegated** administrator to the capabilities they
/// themselves hold, while a system admin stays unrestricted so bootstrap and repair remain operable.
/// Carrying the reason keeps that a single authorization pass instead of re-asking `is_system_admin`
/// after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrantAuthority {
    /// Gating-team owner. Unrestricted: may confer capabilities they do not personally hold.
    SystemAdmin,
    /// Holds `can_grant` on the subject (or owns it). Bound by attenuation.
    Delegated,
    /// May not administer grants on this subject at all.
    None,
}

/// Bool projection of [`grant_authority`], kept as the seam `admin_ledger_service`'s read gate calls
/// (spec 2026-07-16 §5) — it only needs "could you perform the act", not which arm allowed it.
pub(crate) async fn can_administer_grant(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<bool> {
    Ok(grant_authority(pool, caller, subject_table, subject_id).await? != GrantAuthority::None)
}

/// Grant-administration gate: a system admin OR a holder of `can_grant` on the subject (the general
/// `can(...,'grant',...)` seam). This is a DIFFERENT axis from authoring — authoring stays wholly
/// explicit (D3b §3.E), while grant-administration admits admins so pre-existing maps (no seeded
/// `can_grant` holder) and repair stay operable.
///
/// `pub(crate)` for `admin_ledger_service`, whose READ gate mirrors this WRITE gate by CALLING it
/// through [`can_administer_grant`] (spec 2026-07-16 §5): if you could perform the act, you may read
/// the record of it. Restating the predicate there would be a second copy of the policy that drifts
/// from the gate it exists to mirror — tighten this fn and the ledger's read gate tightens with it.
///
/// Note this answers *may you administer grants here at all*. It does NOT bound WHICH capabilities
/// may be conferred — that is attenuation, and both belong to one decision:
/// [`authorize_capability_grant`]. Every grant sink should call that, not this.
pub(crate) async fn grant_authority(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<GrantAuthority> {
    if is_system_admin(pool, caller).await? {
        return Ok(GrantAuthority::SystemAdmin);
    }
    // Structural escalation guard (plan Task 5b.4). `require_cogmap_write_admin` exists to keep the
    // reserved L0 kernel and gating-team-joined maps admin-only, but the grant path never consulted
    // it — so a non-admin `can_grant` holder could mint `can_write` on the kernel, reaching by the
    // grant axis exactly what the write axis forbids. `machine_authz`'s own tests seed such a row,
    // so the state is reachable, not hypothetical. Admins already returned above, so a map in the
    // admin-only regime denies here regardless of any `can_grant` the caller holds.
    if subject_table == "kb_cogmaps"
        && cogmap_write_requires_admin(pool, CogmapId(subject_id)).await?
    {
        return Ok(GrantAuthority::None);
    }
    Ok(
        if profile_can_grant(pool, caller, subject_table, subject_id).await? {
            GrantAuthority::Delegated
        } else {
            GrantAuthority::None
        },
    )
}

/// The four capabilities a grant can carry. A closed set with a fixed SQL spelling — an enum rather
/// than bare `&str` literals so a typo is a compile error and the set cannot silently grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessAction {
    Read,
    Write,
    Delete,
    Grant,
}

impl AccessAction {
    /// The `p_action` spelling `can(...)` dispatches on (`profile_explicit_grant`'s CASE arms).
    const fn as_sql(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
            Self::Grant => "grant",
        }
    }
}

/// Does the caller hold `action` on the subject? The general `can(...)` probe — `profile_can_grant`
/// is the `Grant` specialization, kept because it reads better at its call sites.
pub(crate) async fn profile_can(
    pool: &PgPool,
    caller: ProfileId,
    action: AccessAction,
    subject_table: &str,
    subject_id: Uuid,
) -> ApiResult<bool> {
    let ok = sqlx::query_scalar!(
        "SELECT can('kb_profiles', $1, $2, $3, $4)",
        *caller,
        action.as_sql(),
        subject_table,
        subject_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}

/// The capability set a grant would confer. A struct rather than four positional bools so the two
/// call sites cannot silently transpose them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RequestedCapabilities {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub grant: bool,
}

impl From<&GrantCapabilityRequest> for RequestedCapabilities {
    fn from(r: &GrantCapabilityRequest) -> Self {
        Self {
            read: r.can_read,
            write: r.can_write,
            delete: r.can_delete,
            grant: r.can_grant,
        }
    }
}

/// **The** authorization decision for "may `caller` confer this capability set on this subject?" —
/// authority arm plus attenuation, in one pass.
///
/// This is the single policy for EVERY grant sink, called and never restated. Two sinks exist and
/// they must not drift: the human path (`grant_capability`) and the machine path
/// (`machine_authz::contain_reach` → `apply_reach`). They already drifted once — 5b.3/5b.4 hardened
/// the human path while the machine path kept gating on `can_grant` alone, which let a
/// `read+grant`-without-write holder provision a machine carrying `can_write` and thereby command a
/// principal holding write they could never hold themselves. Laundering by proxy. Hence one helper.
///
/// Semantics:
/// - **SystemAdmin** — unrestricted. Bootstrap and repair would otherwise deadlock: there would be
///   no way to mint the first holder of any capability.
/// - **Delegated** — attenuating: every capability conferred must be one the caller already holds
///   on this subject (`conferred ⊆ held`). Self-grant is neutralized by the same rule, since the
///   check never consults who the principal is.
/// - **None** — denied, which is also where the L0/gating-map guard lands.
pub(crate) async fn authorize_capability_grant(
    pool: &PgPool,
    caller: ProfileId,
    subject_table: &str,
    subject_id: Uuid,
    caps: RequestedCapabilities,
) -> ApiResult<()> {
    match grant_authority(pool, caller, subject_table, subject_id).await? {
        GrantAuthority::None => Err(ApiError::Forbidden),
        GrantAuthority::SystemAdmin => Ok(()),
        GrantAuthority::Delegated => {
            for (requested, action) in [
                (caps.read, AccessAction::Read),
                (caps.write, AccessAction::Write),
                (caps.delete, AccessAction::Delete),
                (caps.grant, AccessAction::Grant),
            ] {
                if requested
                    && !profile_can(pool, caller, action, subject_table, subject_id).await?
                {
                    return Err(ApiError::Forbidden);
                }
            }
            Ok(())
        }
    }
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
    profile_can(pool, caller, AccessAction::Grant, subject_table, subject_id).await
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
pub async fn insert_grant(
    conn: &mut sqlx::PgConnection,
    p: &InsertGrantParams,
    emitter: EntityId,
) -> ApiResult<bool> {
    // The upsert + `grant_created` event, one txn, via the SQL chokepoint `_admin_grant_created`
    // (migrations/20260718000010). `emitter` is the acting entity, resolved from the gated caller.
    // Correlation self-roots — there is no sibling event to fuse with in any grant path, and the
    // SQL fn's `p_correlation` defaults NULL. Returns whether the row was freshly inserted (`xmax = 0`
    // inside the fn); an upsert that only CHANGED capabilities returns false yet still fires the
    // event, carrying `previous`, so a real authority change is never silently dropped.
    Ok(sqlx::query_scalar!(
        r#"SELECT _admin_grant_created($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) AS "inserted!""#,
        emitter.uuid(),
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
    .await?)
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
    revoker: ProfileId,
    emitter: EntityId,
) -> ApiResult<bool> {
    // The DELETE + `grant_revoked` event, one txn, via the SQL chokepoint `_admin_grant_revoked`.
    // Emits ONLY when a row was actually removed — a no-op revoke is not an admin act, and the ledger
    // is append-only so a spurious event is immortal. `revoker` is the acting profile; `emitter` its
    // entity. Correlation self-roots (SQL `p_correlation` defaults NULL). Returns whether a row was
    // removed (absent ⇒ false, idempotent no-op).
    Ok(sqlx::query_scalar!(
        r#"SELECT _admin_grant_revoked($1,$2,$3,$4,$5,$6) AS "deleted!""#,
        emitter.uuid(),
        subject_table,
        subject_id,
        principal_table,
        principal_id,
        revoker.uuid(),
    )
    .fetch_one(&mut *conn)
    .await?)
}

/// Mint/update one access grant. Auth before write: `can_administer_grant`. The DB coherence CHECK
/// (`write|delete|grant ⇒ read`) is the integrity backstop. Idempotent upsert — `granted=false` when
/// the row already existed and was updated in place.
pub async fn grant_capability(
    pool: &PgPool,
    caller: ProfileId,
    req: &GrantCapabilityRequest,
) -> ApiResult<GrantOutcome> {
    // Auth before writes. The one shared decision — authority arm + attenuation — also called by
    // the machine path (`machine_authz::contain_reach`), so the two sinks cannot drift. Revocation
    // is deliberately NOT attenuated: de-escalation must never be harder than escalation, or a
    // grant becomes unwithdrawable.
    authorize_capability_grant(pool, caller, &req.subject_table, req.subject_id, req.into())
        .await?;
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
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
        emitter,
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
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut conn = pool.acquire().await?;
    let revoked = delete_grant(
        &mut conn,
        &req.subject_table,
        req.subject_id,
        &req.principal_table,
        req.principal_id,
        caller,
        emitter,
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
    if !cogmap_write_requires_admin(pool, cogmap_id).await? {
        return Ok(()); // gate doesn't apply to non-reserved, non-root-team cogmaps
    }
    if is_system_admin(pool, profile_id).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// The **structural** half of [`require_cogmap_write_admin`], caller-independent: does this cogmap
/// sit in the admin-only regime at all (reserved L0 kernel, or joined to the gating team)?
///
/// Extracted so `can_administer_grant` can consult the SAME condition without either restating the
/// query — a second copy of the policy is a copy that drifts from the gate it exists to mirror — or
/// swallowing a genuine DB error as a denial (which is what reusing the `Result`-returning form via
/// `.is_err()` would do).
pub(crate) async fn cogmap_write_requires_admin(
    pool: &PgPool,
    cogmap_id: CogmapId,
) -> ApiResult<bool> {
    if cogmap_id == L0_KERNEL_COGMAP {
        return Ok(true); // unconditional, independent of `gating_team_slug` (fail-CLOSED)
    }
    Ok(sqlx::query_scalar!(
        "SELECT EXISTS( \
           SELECT 1 FROM kb_team_cogmaps tc \
             JOIN kb_teams t ON t.id = tc.team_id \
             JOIN kb_system_settings s ON t.slug = s.gating_team_slug \
            WHERE tc.cogmap_id = $1 )",
        *cogmap_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false))
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

/// Admin-authority read of the FULL settings (admin-authz enclosure, spec §3.4). The proof goes on
/// this admin *act*, never on the shared `get_system_settings` reader — that reader also backs the
/// public route and internal callers (`promote_admin`, `update_system_settings`), so gating it would
/// deny the public path. The `_admin` param is the capability; the body just delegates.
pub async fn admin_get_settings(pool: &PgPool, _admin: &SystemAdmin) -> ApiResult<SystemSettings> {
    get_system_settings(pool).await
}

/// Admin-only partial update of the singleton `kb_system_settings` row.
///
/// COALESCE semantics: each `Some` field overwrites its column; each `None`
/// leaves the column unchanged. `access_mode` is no longer writable — it was
/// retired as a control (spec §14 / D18); the column survives read-only until
/// Phase 2 drops it, so this function never touches it.
///
/// One guard survives, decoupled from the retired mode: if a `gating_team_slug`
/// is being set, the team must exist. That slug's ownership confers a system
/// admin (`is_system_admin` reads governance keyed on it), so pointing it at a
/// nonexistent team would silently break admin resolution. This is *not* the old
/// lockout guard — under Task 7's repoint `has_system_access` reads standing, so
/// a null slug can no longer lock anyone out of the instance.
pub async fn update_system_settings(
    pool: &PgPool,
    _admin: &SystemAdmin,
    req: &UpdateSettingsRequest,
) -> ApiResult<SystemSettings> {
    // If a gating team is being set, it must name a real team.
    if let Some(ref slug) = req.gating_team_slug {
        let exists: bool = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM kb_teams WHERE slug = $1)",
            slug
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);
        if !exists {
            return Err(ApiError::BadRequest(format!(
                "gating_team_slug '{slug}' does not exist — create the team first"
            )));
        }
    }

    let row = sqlx::query_as!(
        SystemSettings,
        r#"
        UPDATE kb_system_settings
           SET gating_team_slug   = COALESCE($1, gating_team_slug),
               instance_name      = COALESCE($2, instance_name),
               terms_version      = COALESCE($3, terms_version),
               terms_resource_uri = COALESCE($4, terms_resource_uri),
               updated            = now()
         WHERE id = 1
        RETURNING id, access_mode, gating_team_slug, terms_version,
                  terms_resource_uri, instance_name, updated
        "#,
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
/// not the enum). Auth is the `&SystemAdmin` proof (admin-authz enclosure, spec §3); the promoting
/// admin (`admin.actor()`) is recorded as the actor on the governance grant and standing transition.
pub async fn promote_admin(
    pool: &PgPool,
    admin: &SystemAdmin,
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

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

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
    .fetch_one(&mut *tx)
    .await?;

    // D11: admin-ness IS a governance grant, and the invariant "admin implies Approved" is
    // maintained here, at promotion (is_system_admin never ANDs standing in at read time). So a
    // promotion writes both, atomically with the team row: the governance grant that makes
    // `is_system_admin` true, and — unless the profile is already `approved` — the standing that
    // makes `has_system_access` true. Without the standing half a promoted admin would pass the
    // admin predicate yet fail the front door.
    sqlx::query_scalar!(
        "SELECT principal_governance_set($1, true, $2, 'system admin promotion')",
        profile_id,
        Some(*admin.actor()),
    )
    .fetch_one(&mut *tx)
    .await?;

    let current: Option<String> = sqlx::query_scalar!(
        "SELECT state FROM kb_principal_standing WHERE profile_id = $1",
        profile_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if current.as_deref() != Some("approved") {
        sqlx::query_scalar!(
            "SELECT principal_standing_apply($1,'approve','approved',$2,'system admin promotion')",
            profile_id,
            Some(*admin.actor()),
        )
        .fetch_one(&mut *tx)
        .await?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// The admin standing acts (Task 13)
//
// The five admin-authority acts of the machine, one thin service wrapper each. Every one routes
// through `standing_service::apply`, so it inherits the transition table (a refused act — Revoke on
// a never-approved principal, Reactivate on a live one — is rejected there with a reason, never here)
// and, once they land, Task 15's demotion hook and Task 17's typed refusal. Reject is not among them:
// it is a join-request decision, handled atomically inside `review_request` (D14).
//
// The gate is the `&SystemAdmin` proof each act requires (admin-authz enclosure, spec §3.2): its
// presence in the signature IS the authorization requirement, minted once by `require_system_admin`
// at the surface and read via `admin.actor()`. The old private
// `require_system_admin(pool, actor) -> ApiResult<()>` per-act gate is gone — an ungated call path is
// now a compile error, not a forgotten `.await?`. Both surfaces (temper-api today, temper-mcp when
// parity lands) inherit the gate by construction, and a service caller can never reach
// `ActorAuthority::Admin` without holding the proof. That is the F-3 posture the
// `audit-handler-authz-drift` tripwire pins: authorization the service itself enforces.
// ---------------------------------------------------------------------------

/// Approve a principal directly — the machine/direct-grant door, legal from `Denied` (D14) and
/// `Revoked` (D16) as well as `Requested`. Distinct from `review_request`'s approval, which also
/// enrolls a *human requester* into the auto-join pool; a direct grant confers standing only.
pub async fn admin_approve(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
) -> ApiResult<()> {
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject,
            act: Act::Approve,
            actor: Some(admin.actor()),
            authority: ActorAuthority::Admin,
        },
    )
    .await?;
    Ok(())
}

/// Revoke a principal's admission. Legal only from `Approved` (§6). `reason` rides the log and the
/// ledger, and a later `RequestReview`'s reviewer needs it (D15) — which is why it is required.
pub async fn admin_revoke(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
    reason: String,
) -> ApiResult<()> {
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject,
            act: Act::Revoke { reason },
            actor: Some(admin.actor()),
            authority: ActorAuthority::Admin,
        },
    )
    .await?;
    Ok(())
}

/// Deactivate a principal from any live state (§6).
pub async fn admin_deactivate(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
) -> ApiResult<()> {
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject,
            act: Act::Deactivate,
            actor: Some(admin.actor()),
            authority: ActorAuthority::Admin,
        },
    )
    .await?;
    Ok(())
}

/// Reactivate a deactivated principal, restoring its prior standing (§5). `prior: None` — the seam
/// reads the prior state from the log and refuses rather than guesses.
pub async fn admin_reactivate(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
) -> ApiResult<()> {
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject,
            act: Act::Reactivate { prior: None },
            actor: Some(admin.actor()),
            authority: ActorAuthority::Admin,
        },
    )
    .await?;
    Ok(())
}

/// Demote a system admin — revoke the governance grant (D10 / §9). The manual twin of `promote`, and
/// the deliberate counterpart to demotion-by-transition (Revoke/Deactivate demote automatically in
/// `standing_service::apply`). Unlike `promote` it is **not** team-scoped: governance is keyed on the
/// profile alone, so it takes no team. Idempotent — a no-op on a profile that holds no grant.
///
/// Governance-only: it never touches standing. A demoted admin keeps its access; it just may no
/// longer change the rules. The `&SystemAdmin` proof is the gate (F-3), so both surfaces enforce it
/// identically.
pub async fn demote_admin(pool: &PgPool, admin: &SystemAdmin, subject: ProfileId) -> ApiResult<()> {
    sqlx::query_scalar!(
        "SELECT principal_governance_set($1, false, $2, 'system admin demotion')",
        *subject,
        *admin.actor(),
    )
    .fetch_one(pool)
    .await?;
    Ok(())
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
/// Fires `Act::Request` (Denied → Requested) before writing the request row, so an illegal request
/// (from Revoked or Approved) is refused first. `requested` standing is now the duplicate guard
/// (D12); the old open-mode rejection is gone — under D11 a request is legitimate in any mode.
pub async fn create_join_request(
    pool: &PgPool,
    params: CreateJoinRequestParams,
) -> ApiResult<JoinRequest> {
    // Resolve the request's target FIRST. These are READS, so doing them before any standing write
    // keeps auth-before-writes honest: an unconfigured gating team must fail BEFORE standing moves,
    // or a Denied principal would land in `Requested` with no request row and no legal retry
    // (Request from Requested is illegal). Under D11/D18 the request is legitimate in any mode — the
    // old open-mode rejection made an `open` instance a dead end; `access_mode` is not consulted.
    let settings = get_system_settings(pool).await?;
    let gating_slug = settings
        .gating_team_slug
        .ok_or_else(|| ApiError::Internal("System has no gating team configured".to_string()))?;
    // Resolve team ID from slug (substrate `kb_teams` has no `is_active`).
    let team_id = sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", gating_slug,)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("Gating team '{gating_slug}' not found")))?;

    // Now the standing transition — the first write. An illegal Request (from Revoked, from
    // Approved) refuses here, before the request row exists (auth before writes). The refusal rides
    // `standing_service::apply`'s interim `BadRequest` (Task 17 upgrades it to a typed 403).
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject: params.profile_id,
            act: Act::Request,
            actor: Some(params.profile_id),
            authority: ActorAuthority::SelfPrincipal,
        },
    )
    .await?;

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
    // Standing first (auth before writes): Withdraw is legal only from `Requested` (§6), so a
    // principal with nothing pending is refused here rather than reaching the row UPDATE.
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject: profile_id,
            act: Act::Withdraw,
            actor: Some(profile_id),
            authority: ActorAuthority::SelfPrincipal,
        },
    )
    .await?;

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

/// Parameters for a review request (spec D15 — a revoked principal asking for reconsideration).
pub struct CreateReviewRequestParams {
    pub profile_id: ProfileId,
    pub message: Option<String>,
}

/// Ask an admin to reconsider a revocation (spec D15). Fires `Act::RequestReview`, which validates
/// the principal is `Revoked` and MOVES NOTHING, then records the review as an inbox signal in
/// `kb_principal_review_requests`. The review is NEVER read by the admission decision — a revoked
/// principal is refused whether or not one is pending. Its own partial unique index
/// (`idx_principal_review_one_open`) guards against duplicate open reviews (D15 obligation 2).
pub async fn create_review_request(
    pool: &PgPool,
    params: CreateReviewRequestParams,
) -> ApiResult<()> {
    // Standing gate first (auth before writes): RequestReview is legal only from `Revoked` (§6).
    // It moves nothing — the marker's whole point is that reconsideration cannot launder a
    // revocation (D15). An illegal call refuses before any review row exists.
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject: params.profile_id,
            act: Act::RequestReview,
            actor: Some(params.profile_id),
            authority: ActorAuthority::SelfPrincipal,
        },
    )
    .await?;

    sqlx::query!(
        "INSERT INTO kb_principal_review_requests (profile_id, message) VALUES ($1, $2)",
        *params.profile_id,
        params.message,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// List pending join requests with profile info (admin view). The `_admin` proof is the capability
/// (admin-authz enclosure, spec §3.2).
pub async fn list_pending_requests(
    pool: &PgPool,
    _admin: &SystemAdmin,
) -> ApiResult<Vec<JoinRequestWithProfile>> {
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

/// Parameters for reviewing (approving/rejecting) a join request. The reviewer is no longer carried
/// here: it is the authorizing admin (`admin.actor()` on [`review_request`]), so a caller can no
/// longer supply a reviewer id that disagrees with who actually authorized the decision.
pub struct ReviewRequestParams {
    pub request_id: Uuid,
    pub decision: JoinRequestStatus,
    pub decision_note: Option<String>,
}

/// Approve or reject a join request. On approval, atomically insert the
/// substrate-shaped team membership row (no `id`/`joined_at`/`invited_by_profile_id`;
/// reviewer attribution survives on `kb_join_requests.reviewed_by_profile_id`).
///
/// Auth is the `&SystemAdmin` proof (admin-authz enclosure, spec §3.2); the reviewer recorded on the
/// decision and the standing transition is `admin.actor()`.
pub async fn review_request(
    pool: &PgPool,
    admin: &SystemAdmin,
    params: ReviewRequestParams,
) -> ApiResult<JoinRequest> {
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
        *admin.actor(),
        params.decision_note,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    // On approval, grant the requester access. Under D11 access IS an `approved`
    // `kb_principal_standing` row (`has_system_access` reads nothing else), so the decision and the
    // standing it confers must be one transaction. `Approve` is legal from the requester's born
    // `Denied` state (transition.rs, D14); the raw committer on the tx is the machine-door pattern —
    // it keeps the standing write atomic with the decision, which `standing_service::apply` (a
    // `&PgPool` call) could not. `has_system_access` is now true, so `ensure_auto_join_memberships`
    // below (ordered after) does its enrollment rather than no-op.
    if params.decision == JoinRequestStatus::Approved {
        sqlx::query_scalar!(
            "SELECT principal_standing_apply($1,'approve','approved',$2,$3)",
            row.requesting_profile_id,
            *admin.actor(),
            params.decision_note,
        )
        .fetch_one(&mut *tx)
        .await?;

        // Retain the gating-team membership the pre-D11 model wrote (harmless team-role churn now
        // that access rides standing, not this row) so team-scoped visibility is unchanged.
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

        // Enroll the now-approved profile into the rest of the auto-join "everyone" pool.
        sqlx::query!(
            "SELECT ensure_auto_join_memberships($1)",
            row.requesting_profile_id,
        )
        .execute(&mut *tx)
        .await?;
    } else {
        // Rejection returns standing to `Denied` so the principal may re-request (spec §5;
        // `join_request_rejection_allows_resubmit` pins this). Raw on the tx (machine-door pattern),
        // atomic with the decision, matching Approve; `Reject` is legal from the requester's
        // `Requested` state (transition.rs). Rejection is deliberately NOT a standing state of its
        // own — the request record keeps the `decision_note`.
        sqlx::query_scalar!(
            "SELECT principal_standing_apply($1,'reject','denied',$2,$3)",
            row.requesting_profile_id,
            *admin.actor(),
            params.decision_note,
        )
        .fetch_one(&mut *tx)
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
