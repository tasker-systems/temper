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

use crate::authz::{TwoSidedAuthority, TwoSidedScope};
use crate::error::{ApiError, ApiResult};
use crate::services::team_service;
use temper_core::context_ref::{ContextOwnerRef, ContextRef};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_workflow::operations::sluggify;

pub use temper_core::types::context::{
    ContextCreateRequest, ContextRow, ContextRowWithCounts, InheritedReadGrant, InheritedShare,
    ReassignContextOutcome, ReassignContextRequest, ShareContextOutcome, ShareContextRequest,
    UnshareContextOutcome,
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

/// Does `caller` administer the context — i.e. own it, or manage its owning team?
///
/// A context is owned by `(owner_table, owner_id)`: a profile owns it directly (caller ==
/// owner), while a team-owned context is administered by anyone who `can_manage` that owning
/// team (Owner|Maintainer). A missing context resolves to `false` — the subsequent
/// [`ensure_context_and_team_exist`] check turns that into a clean `NotFound`.
///
/// `pub(crate)` for one consumer: it is the **object-side probe** of
/// `crate::authz::TwoSidedAuthority` — the single question that gate's context arm asks and its
/// cogmap arm does not. It stays defined here, next to the ownership shape it reads, and is called
/// from there rather than restated.
pub(crate) async fn caller_administers_context(
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
/// Auth before writes: the two-sided `crate::authz::TwoSidedAuthority` gate — system-admin, OR
/// the caller administers the context AND manages the target team (owner/maintainer), the same
/// gate `cogmap_service` binds through (its structural sibling; issue #367 landed the relaxation
/// the interim admin-only gate had deferred). Idempotent — `INSERT … ON CONFLICT DO NOTHING`;
/// `shared: false` when it already existed.
pub async fn share(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    req: &ShareContextRequest,
) -> ApiResult<ShareContextOutcome> {
    crate::authz::authorize::<TwoSidedAuthority>(
        pool,
        caller,
        TwoSidedScope::context(context_id, req.team_id),
    )
    .await?;
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
/// (the same `crate::authz::TwoSidedAuthority` gate).
pub async fn unshare(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<UnshareContextOutcome> {
    crate::authz::authorize::<TwoSidedAuthority>(
        pool,
        caller,
        TwoSidedScope::context(context_id, team_id),
    )
    .await?;
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

/// Transfer a context's ownership to a team — the single path to shared *authorship*.
///
/// A personal context stays personal until transferred; read-sharing (`share`) never grants
/// write. Binding the context to a team makes its members (with an authoring role) able to
/// author its resources via the container-write cascade. The owner change is event-sourced
/// (`context_reassigned`) through the substrate writes layer.
///
/// Auth before writes: the two-sided `crate::authz::TwoSidedAuthority` gate — system-admin, OR
/// the caller administers the context AND manages the target team (owner/maintainer), and the
/// target is not the gating/root team. **This call site is why that last clause survives**: a
/// reassign into the root team is a transfer of ownership, and `context_reassign` forbids it in
/// plpgsql independently, so the Rust gate must refuse it too or one act gets two error paths.
/// Idempotent: a context already owned by `to_team_id` returns `reassigned: false` without
/// emitting. A slug collision under the new owner is a `Conflict` (the
/// `UNIQUE(owner_table, owner_id, slug)` constraint is the backstop).
pub async fn reassign(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    to_team_id: uuid::Uuid,
) -> ApiResult<ReassignContextOutcome> {
    crate::authz::authorize::<TwoSidedAuthority>(
        pool,
        caller,
        TwoSidedScope::context(context_id, to_team_id),
    )
    .await?;
    ensure_context_and_team_exist(pool, context_id, to_team_id).await?;

    // Current owner — for the audit fields, idempotency, and the slug-collision check.
    let cur = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!", slug
             FROM kb_contexts WHERE id = $1"#,
        context_id,
    )
    .fetch_one(pool)
    .await?;

    // Read-reach the new owner inherits — surfaced in the outcome, never swept (spec D3).
    let (inherited_shares, inherited_read_grants) = inherited_reach(pool, context_id).await?;

    if cur.owner_table == "kb_teams" && cur.owner_id == to_team_id {
        return Ok(ReassignContextOutcome {
            context_id,
            owner_ref: team_owner_ref(pool, to_team_id).await?,
            reassigned: false,
            inherited_shares,
            inherited_read_grants,
        });
    }

    // The slug must be unique under the NEW owner — 409 rather than a silent re-slug or an
    // opaque UNIQUE violation surfacing from the projector.
    let collision = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_contexts
             WHERE owner_table = 'kb_teams' AND owner_id = $1 AND slug = $2) AS "e!""#,
        to_team_id,
        cur.slug,
    )
    .fetch_one(pool)
    .await?;
    if collision {
        return Err(ApiError::Conflict(format!(
            "team already owns a context with slug '{}'; rename before transferring",
            cur.slug
        )));
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::reassign_context_with(
        pool,
        temper_substrate::ids::ContextId::from(context_id),
        (cur.owner_table.as_str(), cur.owner_id),
        ("kb_teams", to_team_id),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(map_reassign_write_err)?;

    Ok(ReassignContextOutcome {
        context_id,
        owner_ref: team_owner_ref(pool, to_team_id).await?,
        reassigned: true,
        inherited_shares,
        inherited_read_grants,
    })
}

/// Gather the read-reach a transfer leaves in place: `kb_team_contexts` shares plus explicit
/// `kb_access_grants` context read-grants. Surfaced in the transfer outcome; never swept (D3).
async fn inherited_reach(
    pool: &PgPool,
    context_id: uuid::Uuid,
) -> ApiResult<(Vec<InheritedShare>, Vec<InheritedReadGrant>)> {
    let shares = sqlx::query!(
        r#"SELECT tc.team_id AS "team_id!", t.slug AS "slug!"
             FROM kb_team_contexts tc
             JOIN kb_teams t ON t.id = tc.team_id
            WHERE tc.context_id = $1
            ORDER BY t.slug"#,
        context_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| InheritedShare {
        team_id: r.team_id,
        team_ref: format!("+{}", r.slug),
    })
    .collect();

    let grants = sqlx::query!(
        r#"SELECT g.principal_table AS "principal_table!",
                  g.principal_id    AS "principal_id!",
                  COALESCE(p.handle, t.slug) AS "principal_name!"
             FROM kb_access_grants g
             LEFT JOIN kb_profiles p ON g.principal_table = 'kb_profiles' AND p.id = g.principal_id
             LEFT JOIN kb_teams    t ON g.principal_table = 'kb_teams'    AND t.id = g.principal_id
            WHERE g.subject_table = 'kb_contexts' AND g.subject_id = $1 AND g.can_read
            ORDER BY g.principal_table, g.principal_id"#,
        context_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| {
        let principal_ref = if r.principal_table == "kb_profiles" {
            format!("@{}", r.principal_name)
        } else {
            format!("+{}", r.principal_name)
        };
        InheritedReadGrant {
            principal_table: r.principal_table,
            principal_id: r.principal_id,
            principal_ref,
        }
    })
    .collect();

    Ok((shares, grants))
}

/// Map a substrate write error from `reassign_context_with` to an [`ApiError`].
///
/// The `context_reassign` SQL function raises SQLSTATE `42501` (insufficient_privilege) when its
/// atomic RBAC invariant rejects the write. The `can_share` pre-check returns a clean `Forbidden`
/// on the common path, so this only fires on a TOCTOU change between check and write — but that
/// race should still read as `403`, not `500`. Everything else is a genuine internal error.
fn map_reassign_write_err(e: anyhow::Error) -> ApiError {
    if let Some(sqlx::Error::Database(db)) = e.downcast_ref::<sqlx::Error>() {
        if db.code().as_deref() == Some("42501") {
            return ApiError::Forbidden;
        }
    }
    ApiError::Internal(e.to_string())
}

/// `+team-slug` decorated owner ref for a transfer outcome (mirrors `create`'s CASE).
async fn team_owner_ref(pool: &PgPool, team_id: uuid::Uuid) -> ApiResult<String> {
    let slug = sqlx::query_scalar!("SELECT slug FROM kb_teams WHERE id = $1", team_id)
        .fetch_one(pool)
        .await?;
    Ok(format!("+{slug}"))
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// Seed two profiles, a team, and a context owned by profile 1. Profile 1 is made a system
    /// admin — under D11 a `kb_principal_governance` grant (`is_system_admin` reads governance, not
    /// gating-team ownership). The gating-team config is retained because other tests on this
    /// fixture exercise the `is_gating_team` target guard. Fixture inserts use runtime
    /// `sqlx::query(...)`, not the compile-time macro, per project convention for test-fixture writes.
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
        // Every profile that ACTS needs a `<handle>@web` emitter entity for `resolve_emitter`
        // (real profiles get one on first auth). `share`/`unshare` are non-evented so they never
        // needed it, but the event-sourced `reassign` resolves the caller's emitter — without
        // this, a transfer as `admin` panics in `resolve_emitter`.
        sqlx::query(
            "INSERT INTO kb_entities (name, profile_id) \
             VALUES ('admin@web', $1), ('member@web', $2)",
        )
        .bind(admin)
        .bind(non_admin)
        .execute(pool)
        .await
        .unwrap();

        // `temper-system` is created by migration 20260625000001 — use the existing row.
        let team_id: Uuid =
            sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
                .fetch_one(pool)
                .await
                .unwrap();
        // Promote to `owner` of the gating team (kept for the `is_gating_team` target-guard tests
        // that share this fixture) and point the gating slug at it.
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
        // What confers admin-ness under D11: a governance grant (`is_system_admin`).
        crate::test_support::grant_governance(pool, admin).await;

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

    // ── Context ownership transfer (reassign) ────────────────────────────────
    //
    // The target team must be NON-gating: `can_share` refuses the gating team as a target,
    // so these helpers build a fresh `kb_teams` row (never `temper-system`). A caller must
    // administer the source context AND manage the target team to pass the two-sided gate.

    /// A profile plus its `<handle>@web` emitter entity (required by `resolve_emitter`, which
    /// the event-sourced transfer resolves before firing `context_reassigned`).
    async fn mk_profile_ent(pool: &PgPool, handle: &str) -> ProfileId {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO kb_entities (name, profile_id) VALUES ($1 || '@web', $2)")
            .bind(handle)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        ProfileId::from(id)
    }

    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn add_member(pool: &PgPool, team: Uuid, p: ProfileId, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1, $2, $3::team_role, 'native'::team_member_source) \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(team)
        .bind(*p)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn mk_personal_context(pool: &PgPool, slug: &str, owner: ProfileId) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
             VALUES ($1, $1, 'kb_profiles', $2) RETURNING id",
        )
        .bind(slug)
        .bind(*owner)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn mk_team_context(pool: &PgPool, slug: &str, team: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
             VALUES ($1, $1, 'kb_teams', $2) RETURNING id",
        )
        .bind(slug)
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn mk_homed_resource(pool: &PgPool, ctx: Uuid, owner: ProfileId) -> Uuid {
        let rid: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('r', 'r') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_contexts', $2, $3, $3)",
        )
        .bind(rid)
        .bind(ctx)
        .bind(*owner)
        .execute(pool)
        .await
        .unwrap();
        rid
    }

    async fn owner_of_context(pool: &PgPool, ctx: Uuid) -> (String, Uuid) {
        let row = sqlx::query!(
            r#"SELECT owner_table AS "t!", owner_id AS "i!" FROM kb_contexts WHERE id = $1"#,
            ctx
        )
        .fetch_one(pool)
        .await
        .unwrap();
        (row.t, row.i)
    }

    async fn can_modify(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
        sqlx::query_scalar!(
            r#"SELECT can_modify_resource($1, $2) AS "e!""#,
            *profile,
            resource
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn transfers_personal_context_to_team_and_members_can_author(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await; // context owner + team owner
        let bob = mk_profile_ent(&pool, "bob").await; // plain member
        let wanda = mk_profile_ent(&pool, "wanda").await; // watcher
        let stranger = mk_profile_ent(&pool, "stranger").await; // non-member
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, alice, "owner").await;
        add_member(&pool, acme, bob, "member").await;
        add_member(&pool, acme, wanda, "watcher").await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        // Before transfer: only the owner (alice) can author; team members cannot.
        assert!(can_modify(&pool, alice, r).await);
        assert!(!can_modify(&pool, bob, r).await);

        let outcome = reassign(&pool, alice, ctx, acme).await.expect("transfer");
        assert!(outcome.reassigned);
        assert_eq!(outcome.owner_ref, "+acme");
        assert_eq!(
            owner_of_context(&pool, ctx).await,
            ("kb_teams".to_string(), acme)
        );

        // After transfer: authoring-role members can author; watcher cannot; non-member cannot.
        assert!(can_modify(&pool, bob, r).await, "member can author");
        assert!(
            !can_modify(&pool, wanda, r).await,
            "watcher stays read-only"
        );
        assert!(
            !can_modify(&pool, stranger, r).await,
            "non-member unaffected"
        );
        assert!(
            can_modify(&pool, alice, r).await,
            "transferrer retains write"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn transfer_is_idempotent(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, alice, "owner").await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;

        let first = reassign(&pool, alice, ctx, acme).await.unwrap();
        assert!(first.reassigned);
        let second = reassign(&pool, alice, ctx, acme).await.unwrap();
        assert!(!second.reassigned, "already team-owned → no-op");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn non_context_admin_forbidden(pool: PgPool) {
        // mallory manages the target team but does NOT own alice's context.
        let alice = mk_profile_ent(&pool, "alice").await;
        let mallory = mk_profile_ent(&pool, "mallory").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, mallory, "owner").await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;

        let err = reassign(&pool, mallory, ctx, acme).await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn non_target_manager_forbidden(pool: PgPool) {
        // alice owns the context but is only a plain member of the target team.
        let alice = mk_profile_ent(&pool, "alice").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, alice, "member").await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;

        let err = reassign(&pool, alice, ctx, acme).await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn gating_team_target_forbidden(pool: PgPool) {
        // The gating-team refusal applies to non-admins — a system admin bypasses `can_share`
        // entirely (mirrors context_share_e2e's `share_into_gating_team_denied_for_non_admin`).
        // So use a non-admin who owns the context AND even maintains the gating team.
        let (_admin, _member, gating_team, _ctx) = seed_admin_team_context(&pool).await;
        let alice = mk_profile_ent(&pool, "alice").await;
        let alice_ctx = mk_personal_context(&pool, "proj", alice).await;
        add_member(&pool, gating_team, alice, "maintainer").await;

        let err = reassign(&pool, alice, alice_ctx, gating_team)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn slug_collision_is_conflict(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, alice, "owner").await;
        // The team already owns a context with slug "proj"; alice's personal one collides.
        mk_team_context(&pool, "proj", acme).await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;

        let err = reassign(&pool, alice, ctx, acme).await.unwrap_err();
        assert!(matches!(err, ApiError::Conflict(_)));
        assert_eq!(
            owner_of_context(&pool, ctx).await,
            ("kb_profiles".to_string(), *alice),
            "owner unchanged on conflict"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn system_admin_can_transfer_any_context(pool: PgPool) {
        // The seeded admin is a system admin (owner of the gating team). Transfer a context
        // it owns to a fresh non-gating team it also owns.
        let (admin, _member, _gating, context_id) = seed_admin_team_context(&pool).await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, admin, "owner").await;

        let outcome = reassign(&pool, admin, *context_id, acme)
            .await
            .expect("admin transfer");
        assert!(outcome.reassigned);
        assert_eq!(
            owner_of_context(&pool, *context_id).await,
            ("kb_teams".to_string(), acme)
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn transfer_emits_context_reassigned_event(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, alice, "owner").await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;
        reassign(&pool, alice, ctx, acme).await.unwrap();

        let n = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
             WHERE t.name = 'context_reassigned' AND (e.payload->>'context_id')::uuid = $1",
            ctx,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(n, 1);
    }

    /// The `<handle>@web` emitter entity id for a profile (what `resolve_emitter` returns and what
    /// `context_reassign` reads back to `kb_entities.profile_id` for its authz).
    async fn emitter_of(pool: &PgPool, p: ProfileId, handle: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 AND name = $2")
            .bind(*p)
            .bind(format!("{handle}@web"))
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// The RBAC gate is an invariant of the `context_reassign` SQL function, not merely a service
    /// pre-check — this is the TOCTOU backstop the review asked for. Calling the function DIRECTLY
    /// (bypassing `can_share`) with an unauthorized emitter must still be rejected, atomically:
    /// mallory manages the target team but does not administer alice's context, so the guard raises
    /// SQLSTATE 42501 and the owner is left untouched.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sql_guard_rejects_unauthorized_emitter_directly(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let mallory = mk_profile_ent(&pool, "mallory").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, mallory, "owner").await;
        let ctx = mk_personal_context(&pool, "proj", alice).await;

        let payload = temper_substrate::payloads::ContextReassigned {
            context_id: ContextId::from(ctx),
            from_owner_table: "kb_profiles".to_string(),
            from_owner_id: *alice,
            to_owner_table: "kb_teams".to_string(),
            to_owner_id: acme,
        };
        let payload_json = serde_json::to_value(&payload).unwrap();
        let emitter = emitter_of(&pool, mallory, "mallory").await;

        let res = sqlx::query_scalar::<_, Uuid>("SELECT context_reassign($1,$2,$3,$4,$5)")
            .bind(&payload_json)
            .bind(emitter)
            .bind(serde_json::json!({}))
            .bind(None::<Uuid>)
            .bind(None::<Uuid>)
            .fetch_one(&pool)
            .await;

        match res.unwrap_err() {
            sqlx::Error::Database(db) => assert_eq!(
                db.code().as_deref(),
                Some("42501"),
                "authz raise must use insufficient_privilege"
            ),
            other => panic!("expected the SQL RBAC guard to raise, got {other:?}"),
        }
        // Atomic: the rejected write leaves ownership untouched (no partial mutation).
        assert_eq!(
            owner_of_context(&pool, ctx).await,
            ("kb_profiles".to_string(), *alice),
            "owner unchanged when the SQL guard rejects"
        );
    }

    /// The system-admin bypass also holds at the SQL layer: the seeded admin (gating-team owner)
    /// may reassign a context it administers directly through the function.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sql_guard_allows_system_admin_directly(pool: PgPool) {
        let (admin, _member, _gating, context_id) = seed_admin_team_context(&pool).await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, admin, "owner").await;

        let payload = temper_substrate::payloads::ContextReassigned {
            context_id,
            from_owner_table: "kb_profiles".to_string(),
            from_owner_id: *admin,
            to_owner_table: "kb_teams".to_string(),
            to_owner_id: acme,
        };
        let payload_json = serde_json::to_value(&payload).unwrap();
        let emitter = emitter_of(&pool, admin, "admin").await;

        sqlx::query_scalar::<_, Uuid>("SELECT context_reassign($1,$2,$3,$4,$5)")
            .bind(&payload_json)
            .bind(emitter)
            .bind(serde_json::json!({}))
            .bind(None::<Uuid>)
            .bind(None::<Uuid>)
            .fetch_one(&pool)
            .await
            .expect("system admin transfer through the SQL function");

        assert_eq!(
            owner_of_context(&pool, *context_id).await,
            ("kb_teams".to_string(), acme)
        );
    }
}
