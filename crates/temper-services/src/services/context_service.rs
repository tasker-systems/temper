//! Context CRUD service over the substrate.
//!
//! Visibility is centralized in the `context_visible_to(principal, context)`
//! SQL function (migration `20260627000001`): own personal context, OR a context
//! owned by a team the principal is a member of, OR a context explicitly shared
//! (`kb_team_contexts`) to one of the principal's teams. Every read/resolve site
//! below calls that one predicate so they cannot drift. `kb_owner_table`/`kb_owner_id`/
//! `updated` are synthesized to the `ContextRow` shape from the substrate's
//! `owner_table`/`owner_id`/`created` columns. Resource counts come from
//! `kb_resource_homes`. Context creation is a plain INSERT (no event emission —
//! product decision 5: contexts are infrastructure).

use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::services::access_service;
use crate::services::team_service;
use temper_core::context_ref::{ContextOwnerRef, ContextRef};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_workflow::operations::sluggify;

pub use temper_core::types::context::{
    ContextCreateRequest, ContextRow, ContextRowWithCounts, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};

/// List all contexts visible to the profile (owned + team-shared), with resource counts.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Vec<ContextRowWithCounts>> {
    let rows = sqlx::query_as!(
        ContextRowWithCounts,
        r#"
        SELECT c.id, c.name,
               c.owner_table AS "kb_owner_table!",
               c.owner_id AS "kb_owner_id!",
               c.created,
               c.created AS "updated!",
               c.slug,
               CASE c.owner_table
                 WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = c.owner_id)
                 ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = c.owner_id)
               END AS "owner_ref!",
               COUNT(rh.resource_id) AS "resource_count!"
          FROM kb_contexts c
          LEFT JOIN kb_resource_homes rh
                 ON rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id
         WHERE context_visible_to($1, c.id)
         GROUP BY c.id, c.name, c.owner_table, c.owner_id, c.created, c.slug
         ORDER BY c.name
        "#,
        *profile_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get a single context by ID, scoped to profile visibility.
pub async fn get_visible(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
) -> ApiResult<ContextRow> {
    sqlx::query_as!(
        ContextRow,
        r#"
        SELECT c.id, c.name,
               c.owner_table AS "kb_owner_table!",
               c.owner_id AS "kb_owner_id!",
               c.created,
               c.created AS "updated!",
               c.slug,
               CASE c.owner_table
                 WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = c.owner_id)
                 ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = c.owner_id)
               END AS "owner_ref!"
          FROM kb_contexts c
         WHERE c.id = $2
           AND context_visible_to($1, c.id)
        "#,
        *profile_id,
        *context_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a context ref to a `ContextId`, gated to what `principal` may see.
///
/// The single source of truth for context resolution. `@me` uses the caller's
/// profile; `@handle`/`+team` resolve the owner then the `(owner, slug)` row;
/// a bare UUID must be visible to the principal.
///
/// Error taxonomy:
/// - `Id`/`Handle`/profile-context miss → `NotFound`
/// - Team non-membership → `Forbidden` (existence of team/context not leaked)
pub async fn resolve_context_ref(
    pool: &PgPool,
    principal: ProfileId,
    r: &ContextRef,
) -> ApiResult<ContextId> {
    match r {
        ContextRef::Id(id) => {
            // Visible-to-principal gate: profile-owned or team-shared.
            let found = sqlx::query_scalar!(
                r#"
                SELECT c.id FROM kb_contexts c
                 WHERE c.id = $2
                   AND context_visible_to($1, c.id)
                "#,
                *principal,
                id
            )
            .fetch_optional(pool)
            .await?;
            found.map(ContextId::from).ok_or(ApiError::NotFound)
        }
        ContextRef::OwnerSlug { owner, slug } => match owner {
            ContextOwnerRef::Me => lookup_profile_context(pool, *principal, slug).await,
            ContextOwnerRef::Handle(handle) => {
                let owner_id =
                    sqlx::query_scalar!("SELECT id FROM kb_profiles WHERE handle = $1", handle)
                        .fetch_optional(pool)
                        .await?
                        .ok_or(ApiError::NotFound)?;
                // Resolve the context, then gate visibility to the principal.
                let cid = lookup_profile_context(pool, owner_id, slug).await?;
                ensure_context_visible(pool, *principal, *cid).await?;
                Ok(cid)
            }
            ContextOwnerRef::Team(team_slug) => {
                let team_id =
                    sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", team_slug)
                        .fetch_optional(pool)
                        .await?
                        .ok_or(ApiError::NotFound)?;
                // Membership gate — non-member gets Forbidden, not NotFound.
                let is_member = sqlx::query_scalar!(
                    r#"SELECT EXISTS(
                         SELECT 1 FROM kb_team_members
                          WHERE team_id = $1 AND profile_id = $2) AS "ok!""#,
                    team_id,
                    *principal
                )
                .fetch_one(pool)
                .await?;
                if !is_member {
                    return Err(ApiError::Forbidden);
                }
                let id = sqlx::query_scalar!(
                    "SELECT id FROM kb_contexts \
                     WHERE owner_table = 'kb_teams' AND owner_id = $1 AND slug = $2",
                    team_id,
                    slug
                )
                .fetch_optional(pool)
                .await?
                .ok_or(ApiError::NotFound)?;
                Ok(ContextId::from(id))
            }
        },
    }
}

/// Look up a profile-owned context by `(owner_id, slug)`.
async fn lookup_profile_context(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    slug: &str,
) -> ApiResult<ContextId> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = $2",
        owner_id,
        slug
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ContextId::from(id))
}

/// Assert that `principal` may see `context_id` (owned or team-shared).
async fn ensure_context_visible(
    pool: &PgPool,
    principal: uuid::Uuid,
    context_id: uuid::Uuid,
) -> ApiResult<()> {
    let visible = sqlx::query_scalar!(
        r#"SELECT context_visible_to($1, $2) AS "ok!""#,
        principal,
        context_id
    )
    .fetch_one(pool)
    .await?;
    if visible {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// Pick a slug for a new context, unique within `(owner_table, owner_id, slug)`.
///
/// Bases the slug on the name; on collision (two distinct names can sluggify to
/// the same value, and the substrate's unique constraint is on the slug, not the
/// name) appends a numeric suffix. The `(owner_table, owner_id, slug)` unique
/// constraint is the backstop against the check-then-insert race.
async fn next_unique_context_slug(
    pool: &PgPool,
    owner_table: &str,
    owner_id: uuid::Uuid,
    name: &str,
) -> ApiResult<String> {
    let base = {
        let s = sluggify(name);
        if s.is_empty() {
            "context".to_owned()
        } else {
            s
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    loop {
        let taken = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM kb_contexts
                 WHERE owner_table = $1 AND owner_id = $2 AND slug = $3
            ) AS "taken!"
            "#,
            owner_table,
            owner_id,
            candidate
        )
        .fetch_one(pool)
        .await?;

        if !taken {
            return Ok(candidate);
        }
        candidate = format!("{base}-{n}");
        n += 1;
    }
}

/// Resolve a context-create request's owner descriptor to `(owner_table, owner_id)`,
/// enforcing the role gate **before** any write (auth-before-writes).
///
/// - `None` / `Me` → the caller's own profile (`kb_profiles`), preserving the
///   pre-Chunk-3 default.
/// - `Team(slug)` → the team (must exist, else `NotFound`); the caller must be
///   `owner`/`maintainer` on it (reuses `team_service`'s role check — no
///   duplicated authz), else `Forbidden`.
/// - `Handle(_)` → `BadRequest`: a profile cannot create a context owned by
///   another profile.
pub async fn resolve_create_owner(
    pool: &PgPool,
    caller: ProfileId,
    owner: Option<&ContextOwnerRef>,
) -> ApiResult<(String, uuid::Uuid)> {
    match owner {
        None | Some(ContextOwnerRef::Me) => Ok(("kb_profiles".to_owned(), *caller)),
        Some(ContextOwnerRef::Team(slug)) => {
            let team_id = sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", slug)
                .fetch_optional(pool)
                .await?
                .ok_or(ApiError::NotFound)?;
            match team_service::role_on_team(pool, team_id, caller).await? {
                Some(role) if team_service::can_manage(role) => {}
                _ => return Err(ApiError::Forbidden),
            }
            Ok(("kb_teams".to_owned(), team_id))
        }
        Some(ContextOwnerRef::Handle(_)) => Err(ApiError::BadRequest(
            "cannot create a context owned by another profile".to_owned(),
        )),
    }
}

/// Create a new context owned by `(owner_table, owner_id)`: a plain INSERT with a
/// generated slug and NO event emission (product decision 5 — contexts are
/// infrastructure). The owner is resolved + role-gated upstream by
/// [`resolve_create_owner`].
///
/// The substrate enforces uniqueness on the generated slug
/// (`(owner_table, owner_id, slug)`), not the name — `next_unique_context_slug`
/// auto-suffixes on collision (scoped to this owner), so two contexts sharing a
/// name coexist under distinct slugs rather than 409ing.
pub async fn create(
    pool: &PgPool,
    owner_table: &str,
    owner_id: uuid::Uuid,
    name: &str,
) -> ApiResult<ContextRow> {
    let id = ContextId::new();
    let slug = next_unique_context_slug(pool, owner_table, owner_id, name).await?;

    let row = sqlx::query_as!(
        ContextRow,
        r#"
        INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, name,
                  owner_table AS "kb_owner_table!",
                  owner_id AS "kb_owner_id!",
                  created,
                  created AS "updated!",
                  slug,
                  CASE owner_table
                    WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = owner_id)
                    ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = owner_id)
                  END AS "owner_ref!"
        "#,
        *id,
        owner_table,
        owner_id,
        slug,
        name
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Assert both `context_id` and `team_id` exist before a `kb_team_contexts` write —
/// otherwise a nonexistent id hits the FK constraint and surfaces as an opaque 500
/// instead of a clean 404. Called AFTER the admin gate (auth stays first) and BEFORE
/// the INSERT/DELETE.
async fn ensure_context_and_team_exist(
    pool: &PgPool,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<()> {
    let context_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_contexts WHERE id = $1) AS "ok!""#,
        context_id
    )
    .fetch_one(pool)
    .await?;
    if !context_exists {
        return Err(ApiError::NotFound);
    }
    let team_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1) AS "ok!""#,
        team_id
    )
    .fetch_one(pool)
    .await?;
    if !team_exists {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Two-sided share/unshare gate — the context-share analogue of
/// `cogmap_service::can_bind`. Allowed IFF `is_system_admin`, OR the caller administers the
/// CONTEXT (see [`caller_administers_context`]) AND may manage the TARGET TEAM (`can_manage`
/// = Owner|Maintainer, direct membership) AND that team is NOT the gating/root team.
///
/// The gating-team exclusion mirrors `can_bind`: sharing into the root team is an
/// instance-level escalation, kept admin-only.
async fn can_share(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<bool> {
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(true);
    }
    if access_service::is_gating_team(pool, team_id).await? {
        return Ok(false);
    }
    let team_ok = matches!(
        team_service::role_on_team(pool, team_id, caller).await?,
        Some(role) if team_service::can_manage(role)
    );
    if !team_ok {
        return Ok(false);
    }
    caller_administers_context(pool, caller, context_id).await
}

/// Does `caller` administer the context — i.e. own it, or manage its owning team?
///
/// A context is owned by `(owner_table, owner_id)`: a profile owns it directly (caller ==
/// owner), while a team-owned context is administered by anyone who `can_manage` that owning
/// team (Owner|Maintainer). A missing context resolves to `false` — the subsequent
/// [`ensure_context_and_team_exist`] check turns that into a clean `NotFound`.
async fn caller_administers_context(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
) -> ApiResult<bool> {
    let Some(owner) = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!"
           FROM kb_contexts WHERE id = $1"#,
        context_id
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(false);
    };
    match owner.owner_table.as_str() {
        "kb_profiles" => Ok(owner.owner_id == *caller),
        "kb_teams" => Ok(matches!(
            team_service::role_on_team(pool, owner.owner_id, caller).await?,
            Some(role) if team_service::can_manage(role)
        )),
        _ => Ok(false),
    }
}

/// Share a context into a team's read-reach (write a `kb_team_contexts` row).
///
/// Auth before writes: the two-sided `can_share` gate — system-admin, OR the caller
/// administers the context AND manages the target team (owner/maintainer), the same shape
/// as `cogmap_service::can_bind` (its structural sibling; issue #367 landed the relaxation
/// the interim admin-only gate had deferred). Idempotent — `INSERT … ON CONFLICT DO NOTHING`;
/// `shared: false` when it already existed.
pub async fn share(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    req: &ShareContextRequest,
) -> ApiResult<ShareContextOutcome> {
    if !can_share(pool, caller, context_id, req.team_id).await? {
        return Err(ApiError::Forbidden);
    }
    ensure_context_and_team_exist(pool, context_id, req.team_id).await?;
    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO kb_team_contexts (context_id, team_id)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        RETURNING context_id
        "#,
        context_id,
        req.team_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(ShareContextOutcome {
        context_id,
        team_id: req.team_id,
        shared: inserted.is_some(),
    })
}

/// Unshare a context from a team (delete the `kb_team_contexts` row). No-op safe.
///
/// Auth before writes: symmetric with [`share`] — a principal who could share may unshare
/// (the same `can_share` gate).
pub async fn unshare(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<UnshareContextOutcome> {
    if !can_share(pool, caller, context_id, team_id).await? {
        return Err(ApiError::Forbidden);
    }
    ensure_context_and_team_exist(pool, context_id, team_id).await?;
    let result = sqlx::query!(
        "DELETE FROM kb_team_contexts WHERE context_id = $1 AND team_id = $2",
        context_id,
        team_id,
    )
    .execute(pool)
    .await?;
    Ok(UnshareContextOutcome {
        context_id,
        team_id,
        unshared: result.rows_affected() > 0,
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// Seed two profiles, a team, and a context owned by profile 1. Profile 1 is made an
    /// `owner` of the `temper-system` gating team (with `kb_system_settings.gating_team_slug`
    /// pointed at it), so `is_system_admin` resolves true for it — mirroring the admin-minting
    /// idiom in `cogmap_authz_test.rs`. Fixture inserts use runtime `sqlx::query(...)`, not the
    /// compile-time macro, per project convention for test-fixture writes.
    async fn seed_admin_team_context(pool: &PgPool) -> (ProfileId, ProfileId, Uuid, ContextId) {
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('admin', 'Admin') \
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let non_admin: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('member', 'Member') \
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();

        // `temper-system` is created by migration 20260625000001 — use the existing row.
        let team_id: Uuid =
            sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
                .fetch_one(pool)
                .await
                .unwrap();
        // The auto-join trigger may already have enrolled the profile as `watcher`
        // (open-mode auto-join on temper-system) — promote it to `owner` so
        // `is_system_admin` (OWNER of the gating team) resolves true.
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner'",
        )
        .bind(team_id)
        .bind(admin)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1",
        )
        .execute(pool)
        .await
        .unwrap();

        let context_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'ctx', 'Ctx') RETURNING id",
        )
        .bind(admin)
        .fetch_one(pool)
        .await
        .unwrap();

        (
            ProfileId::from(admin),
            ProfileId::from(non_admin),
            team_id,
            ContextId::from(context_id),
        )
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn share_is_admin_gated_and_idempotent(pool: PgPool) {
        // Non-admin caller → Forbidden.
        let (admin, non_admin, team_id, context_id) = seed_admin_team_context(&pool).await;
        let req = ShareContextRequest { team_id };
        let denied = share(&pool, non_admin, *context_id, &req).await;
        assert!(matches!(denied, Err(ApiError::Forbidden)));

        // Admin → shares; first call inserts, second is a no-op.
        let first = share(&pool, admin, *context_id, &req).await.unwrap();
        assert!(first.shared);
        let second = share(&pool, admin, *context_id, &req).await.unwrap();
        assert!(!second.shared);

        // Unshare removes it; second unshare is a no-op.
        let u1 = unshare(&pool, admin, *context_id, team_id).await.unwrap();
        assert!(u1.unshared);
        let u2 = unshare(&pool, admin, *context_id, team_id).await.unwrap();
        assert!(!u2.unshared);
    }
}
