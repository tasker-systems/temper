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

use crate::authz::{ContextAdminAuthority, TwoSidedAuthority, TwoSidedScope};
use crate::error::{ApiError, ApiResult};
use crate::services::team_service;
use temper_core::context_ref::{ContextOwnerRef, ContextRef};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_workflow::operations::sluggify;

pub use temper_core::types::context::{
    ContextCreateRequest, ContextRow, ContextRowWithCounts, InheritedReadGrant, InheritedShare,
    ReassignContextOutcome, ReassignContextRequest, RenameContextOutcome, RenameContextRequest,
    ShareContextOutcome, ShareContextRequest, UnshareContextOutcome,
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
    .ok_or_else(|| ApiError::NotFound(CONTEXT_REFUSAL.to_string()))
}

/// **The one refusal every third-party context lookup renders.**
///
/// A caller resolving `@handle/slug` has three ways to fail — no such handle, no such context
/// under it, or one that exists but is not theirs to see — and telling them apart answers
/// questions the caller has no standing to ask: whether a handle names a real profile, and, one
/// guess per request, what another user's private contexts are called. At base all three rendered
/// the bare `Not found` and were indistinguishable for free; carrying a message is what makes the
/// property something to hold deliberately.
///
/// A constant rather than a literal per site, for the reason `FINDING_REFUSAL` gives: four copies
/// of one defence is four defences that drift. `the_three_handle_slug_refusals_are_indistinguishable`
/// asserts the byte-identity directly.
///
/// The `@me` arm is deliberately **not** bound by this and still names the slug — that slug is the
/// caller's own, so echoing it discloses nothing they did not supply.
///
/// `pub(crate)` for its second consumer: `crate::authz::ContextAdminAuthority::denial_for` renders
/// the `Invisible` arm with **this** constant, so an administration refusal is byte-identical to
/// the read refusals above rather than a fourth copy of the same defence.
pub(crate) const CONTEXT_REFUSAL: &str = "context not found or not readable";

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
            found
                .map(ContextId::from)
                .ok_or_else(|| ApiError::NotFound(CONTEXT_REFUSAL.to_string()))
        }
        ContextRef::OwnerSlug { owner, slug } => match owner {
            ContextOwnerRef::Me => lookup_profile_context(pool, *principal, slug).await,
            ContextOwnerRef::Handle(handle) => {
                // All three exits below render `CONTEXT_REFUSAL` and nothing else. An unresolvable
                // handle must not be distinguishable from a resolvable one, or the arm becomes a
                // membership oracle over every handle on the instance.
                let owner_id =
                    sqlx::query_scalar!("SELECT id FROM kb_profiles WHERE handle = $1", handle)
                        .fetch_optional(pool)
                        .await?
                        .ok_or_else(|| ApiError::NotFound(CONTEXT_REFUSAL.to_string()))?;
                // Resolve the context, then gate visibility to the principal. The inner lookup
                // names the slug — right for `@me`, wrong here, where "no such slug" and "exists
                // but not yours" must read alike — so its refusal is rewritten. Only `NotFound` is
                // rewritten; a genuine fault still propagates as itself.
                let cid =
                    lookup_profile_context(pool, owner_id, slug)
                        .await
                        .map_err(|e| match e {
                            ApiError::NotFound(_) => {
                                ApiError::NotFound(CONTEXT_REFUSAL.to_string())
                            }
                            other => other,
                        })?;
                ensure_context_visible(pool, *principal, *cid).await?;
                Ok(cid)
            }
            ContextOwnerRef::Team(team_slug) => {
                let team_id =
                    sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", team_slug)
                        .fetch_optional(pool)
                        .await?
                        .ok_or_else(|| {
                            ApiError::NotFound(format!(
                                "team {team_slug} not found or not readable"
                            ))
                        })?;
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
                .ok_or_else(|| {
                    ApiError::NotFound(format!("context {slug} not found or not readable"))
                })?;
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
    .ok_or_else(|| ApiError::NotFound(format!("context {slug} not found or not readable")))?;
    Ok(ContextId::from(id))
}

/// May `principal` see `context_id` (owned or team-shared)? The predicate, not the assertion.
///
/// The one Rust spelling of the `context_visible_to(principal, context)` probe. Extracted from
/// [`ensure_context_visible`], which keeps its own error rendering and now calls this, because
/// `crate::authz::ContextAdminAuthority` must **distinguish** "visible but not administered"
/// (`403`) from "not visible" (`404`) and so needs the boolean rather than the refusal. Inferring
/// the arm by catching `ensure_context_visible`'s `ApiError` would be worse than the duplication it
/// avoids; a second copy of the SQL would be the duplication itself. One name for the concept.
pub(crate) async fn context_visible(
    pool: &PgPool,
    principal: uuid::Uuid,
    context_id: uuid::Uuid,
) -> ApiResult<bool> {
    let visible = sqlx::query_scalar!(
        r#"SELECT context_visible_to($1, $2) AS "ok!""#,
        principal,
        context_id
    )
    .fetch_one(pool)
    .await?;
    Ok(visible)
}

/// Assert that `principal` may see `context_id` (owned or team-shared).
async fn ensure_context_visible(
    pool: &PgPool,
    principal: uuid::Uuid,
    context_id: uuid::Uuid,
) -> ApiResult<()> {
    if context_visible(pool, principal, context_id).await? {
        Ok(())
    } else {
        Err(ApiError::NotFound(CONTEXT_REFUSAL.to_string()))
    }
}

/// The one canonical spelling of a stored context name: leading/trailing whitespace trimmed,
/// internal runs of whitespace collapsed to a single space.
///
/// **Whitespace and nothing else.** It deliberately does **not** reuse [`sluggify`]'s ASCII fold:
/// a slug is an *address* and may be lossy, a name is a *display label* and may not — `Café` stays
/// `Café`. Two normalizations answering two different questions; sharing an implementation would
/// silently make the display label lossy too.
///
/// Called from **every** path that stores a context name: [`create`], [`rename`], and
/// `connection_service`'s home-context insert. Spec §"Names are canonicalized before they are
/// stored — on both write paths": *"A canonical-form invariant honoured by one of two write paths
/// is not an invariant — rename would be a repair affordance for a hole `create` keeps digging."*
/// `create`'s stored-name behavior therefore changes here, deliberately: a name that used to be
/// persisted verbatim is now persisted canonical. The derived slug is unaffected — `sluggify`
/// already splits on runs of non-alphanumerics, so `"Temper  KB"` and `"Temper KB"` always yielded
/// the same slug; the defect this closes was entirely in the stored `name`.
///
/// **The spec says "both" because it knew of two; disk had three.** `connection_service` births a
/// home context from a free-text `req.name`, and the spec's own argument applies to it verbatim —
/// an invariant honoured by two of three write paths is no more an invariant than one of two. It
/// is `pub(crate)` for that third caller.
pub(crate) fn canonical_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
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
                .ok_or_else(|| {
                    ApiError::NotFound(format!("team {slug} not found or not readable"))
                })?;
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
///
/// The name is stored in `canonical_name` form. This is the second of that helper's two callers
/// and the reason it exists on this path at all: a canonical-form invariant honoured only by
/// [`rename`] would make rename a repair affordance for a hole `create` keeps digging.
pub async fn create(
    pool: &PgPool,
    owner_table: &str,
    owner_id: uuid::Uuid,
    name: &str,
) -> ApiResult<ContextRow> {
    let id = ContextId::new();
    let name = canonical_name(name);
    let slug = next_unique_context_slug(pool, owner_table, owner_id, &name).await?;

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
        return Err(ApiError::NotFound("context not found".to_string()));
    }
    let team_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1) AS "ok!""#,
        team_id
    )
    .fetch_one(pool)
    .await?;
    if !team_exists {
        return Err(ApiError::NotFound("team not found".to_string()));
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
    .map_err(map_context_write_err)?;

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

/// Rename a context — the one act that moves its `(name, slug)` identity pair in place.
///
/// **Rename takes a name; the slug is derived.** There is deliberately no independent slug
/// parameter, and the consequence is stated rather than mitigated: a rename **re-addresses** the
/// context. After renaming `@me/temper` to `"Temper KB"`, `@me/temper` no longer resolves and
/// `@me/temper-kb` does, and every stored `@owner/slug` string held by anyone is stale. That is why
/// the outcome carries the composed `context_ref`.
///
/// **Auth before writes**, and before every read: `ContextAdminAuthority` is last in the spec's
/// refusal table but first in this body. It answers in two dialects — `403` to a caller who reads
/// the context but does not administer it, `404` to one who cannot see it — which is the whole
/// reason `ScopedAuthority::denial_for` exists. The `context_rename` SQL function re-runs the same
/// admission rule as an in-transaction invariant, so this gate is the caller-facing refusal and
/// that one is the atomic backstop (mapped through `map_context_write_err`).
///
/// The remaining refusals, **in order** (spec §"Refusals"):
///
/// 1. an empty derived slug → `400`, a deliberate divergence from [`create`], which falls back to
///    the literal `"context"`. Tolerable at birth, wrong at rename: `--name "!!!"` must not silently
///    re-address a context to `context`. One check covers both cases — an all-whitespace name
///    canonicalizes to empty and an empty name sluggifies to empty.
/// 2. the **canonical name** already equal to the stored one → `200`, `renamed: false`, no event.
/// 3. the derived slug taken by **another** context under the same owner → `409`, a deliberate
///    divergence from [`create`], which auto-suffixes (`notes`, `notes-2`). A rename is a deliberate
///    re-address, and silently landing on `notes-2` gives the caller an address they did not ask for
///    and were not told about. Hence this path calls [`sluggify`] directly and **never**
///    `next_unique_context_slug`, whose two conveniences are exactly the two divergences above.
///
/// **What is compared for the no-op is the canonical NAME, never the derived slug.** Slug-comparison
/// is the natural thing to write and is wrong twice over: a stored name predating canonicalization
/// (`"Temper  KB"`) sluggifies identically to its own repaired form, so slug-comparison would
/// permanently decline the one rename that fixes it; and two genuinely different names can share a
/// slug (`"Temper KB"` / `"Temper-KB"`), so it would swallow a real display-name change and report
/// success. Consequently **a rename whose slug does not move is still a rename**: it writes `name`,
/// emits, and returns `renamed: true` with `from_slug == to_slug` recording that the address stayed
/// put.
pub async fn rename(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
    name: &str,
) -> ApiResult<RenameContextOutcome> {
    crate::authz::authorize::<ContextAdminAuthority>(pool, caller, context_id).await?;

    // The current identity pair plus the already-sigil'd owner ref, in one read. A rename leaves
    // `(owner_table, owner_id)` untouched, so the ref composed from this row is still correct after
    // the write. The `CASE` is the incumbent both-kinds spelling (`:42-46`, `:77-80`, `:377-380`) —
    // `team_owner_ref` is team-only and would `fetch_one`-panic on a profile-owned context.
    //
    // `fetch_optional`, not `fetch_one`: the gate's `SystemAdmin` arm admits without consulting the
    // subject's existence (it probes the caller's governance grant), so a system admin naming a
    // context id that does not exist reaches here. That is the read refusal, not a 500.
    let cur = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!", slug, name,
              CASE owner_table
                WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = owner_id)
                ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = owner_id)
              END AS "owner_ref!"
         FROM kb_contexts WHERE id = $1"#,
        context_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(CONTEXT_REFUSAL.to_string()))?;

    let to_name = canonical_name(name);
    let to_slug = sluggify(&to_name);

    // 1. Refuse rather than re-address: a name with no addressable content gets no address.
    if to_slug.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "name '{name}' has no addressable content; \
             use a name with at least one alphanumeric character"
        )));
    }

    // 2. Idempotent no-op — the canonical NAME, never the slug (see the doc comment above).
    if to_name == cur.name {
        let context_ref = format!("{}/{}", cur.owner_ref, cur.slug);
        return Ok(RenameContextOutcome {
            context_id,
            name: cur.name,
            slug: cur.slug,
            owner_ref: cur.owner_ref,
            context_ref,
            renamed: false,
        });
    }

    // 3. The derived slug must be free under the (unchanged) owner.
    //
    // `AND id <> $4` is load-bearing and is the one line that must NOT be copied from `reassign`'s
    // query (`:583-590`), which has no self-exclusion and correctly needs none — it changes the
    // *owner*, so the row can never match itself. Rename keeps the owner fixed, so without this a
    // name-only rename (canonical name moved, slug did not) reaches here and 409s against its own
    // slug — breaking exactly the legacy-repair case the no-op rule above exists to enable.
    let collision = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_contexts
             WHERE owner_table = $1 AND owner_id = $2 AND slug = $3 AND id <> $4) AS "e!""#,
        cur.owner_table,
        cur.owner_id,
        to_slug,
        context_id,
    )
    .fetch_one(pool)
    .await?;
    if collision {
        // Naming the colliding slug is not a leak: reaching the gate at all means the caller
        // administers this context, so they own it or manage the owning team and can already
        // enumerate that owner's contexts. Modelled on `reassign`'s message (`:591-596`).
        return Err(ApiError::Conflict(format!(
            "{} already owns a context with slug '{to_slug}'; pick another name",
            cur.owner_ref
        )));
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::rename_context_with(
        pool,
        temper_substrate::ids::ContextId::from(context_id),
        (cur.name.as_str(), cur.slug.as_str()),
        (to_name.as_str(), to_slug.as_str()),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(map_context_write_err)?;

    // Composed once, here, from the already-decorated `owner_ref` — never through
    // `decorated_context_ref`, whose `owner_addressable` parameter is the *bare* handle/team slug
    // and would yield `@@handle/slug` from this value.
    let context_ref = format!("{}/{to_slug}", cur.owner_ref);
    Ok(RenameContextOutcome {
        context_id,
        name: to_name,
        slug: to_slug,
        owner_ref: cur.owner_ref,
        context_ref,
        renamed: true,
    })
}

/// The `23505` refusal both context write paths render when they lose the collision race.
///
/// A constant rather than a literal, for the reason [`CONTEXT_REFUSAL`] gives: it is one defence
/// shared by two call sites, and two copies would be two defences that drift. It deliberately names
/// neither the owner nor the slug — unlike the pre-check messages, which have a row in hand, this
/// one is reconstructed from a SQLSTATE and has nothing but the constraint that fired.
const CONTEXT_SLUG_TAKEN: &str = "that owner already holds a context with this slug";

/// Map a substrate write error from `reassign_context_with` / `rename_context_with` to an
/// [`ApiError`]. **One mapper, two call sites** — the mapping is identical for both and would drift
/// if written twice.
///
/// - `42501` (insufficient_privilege) — `context_reassign` and `context_rename` each carry their
///   RBAC gate as an in-transaction invariant, not merely a caller pre-check. The Rust gate renders
///   a clean refusal on the common path, so this arm only fires on a TOCTOU change between check and
///   write; that lost race should still read as `403`, not `500`.
/// - `23505` (unique_violation) — `UNIQUE (owner_table, owner_id, slug)` is the backstop behind both
///   call sites' slug-collision pre-check. **The race path must render the same refusal as the
///   pre-check**, or the caller's experience depends on how quickly they lost the race rather than
///   on the state of the system: the caller who won gets `409` and the caller who lost gets `500`
///   for the same conflict. `reassign` shipped with exactly this hole — a `42501`-only body — and
///   rename's tests are what surfaced it; it is fixed here rather than left beside a correct copy.
///
/// Everything else is a genuine internal error.
fn map_context_write_err(e: anyhow::Error) -> ApiError {
    if let Some(sqlx::Error::Database(db)) = e.downcast_ref::<sqlx::Error>() {
        match db.code().as_deref() {
            Some("42501") => return ApiError::Forbidden,
            Some("23505") => return ApiError::Conflict(CONTEXT_SLUG_TAKEN.to_string()),
            _ => {}
        }
    }
    ApiError::Internal(e.to_string())
}

/// The mapper and the canonicalizer, exercised directly — no database, no race.
///
/// This module is deliberately **not** `test-db`-gated. The `23505` arm exists for a path that only
/// a real interleaving reaches, and nothing in this crate's suites provokes one; a synthetic
/// `sqlx::Error::Database` carrying only a SQLSTATE is the only evidence available for the
/// concurrent half of `one-owner-never-holds-two-of-the-same-address`, so it must at least run in
/// the tier that needs no services.
#[cfg(test)]
mod write_err_mapper_tests {
    use super::*;
    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::fmt;

    /// A [`sqlx::error::DatabaseError`] carrying nothing but a SQLSTATE — the one field
    /// [`map_context_write_err`] reads. Everything else is the trait's minimum.
    #[derive(Debug)]
    struct SqlState(&'static str);

    impl fmt::Display for SqlState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "synthetic database error, SQLSTATE {}", self.0)
        }
    }

    impl StdError for SqlState {}

    impl sqlx::error::DatabaseError for SqlState {
        fn message(&self) -> &str {
            "synthetic database error"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }
        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    /// The shape the substrate hands back: `fire_with`'s `?` converts `sqlx::Error` straight into
    /// `anyhow::Error` with no wrapping, which is what makes the mapper's `downcast_ref` work.
    fn substrate_err(sqlstate: &'static str) -> anyhow::Error {
        anyhow::Error::new(sqlx::Error::Database(Box::new(SqlState(sqlstate))))
    }

    /// The bundled fix, asserted directly. Without this arm a lost collision race renders `500`
    /// where the pre-check renders `409`, so the answer depends on timing rather than on state.
    #[test]
    fn unique_violation_renders_the_same_conflict_the_pre_check_renders() {
        match map_context_write_err(substrate_err("23505")) {
            ApiError::Conflict(msg) => assert_eq!(msg, CONTEXT_SLUG_TAKEN),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    /// The incumbent arm, unchanged: the in-transaction RBAC invariant's raise reads as `403`.
    #[test]
    fn insufficient_privilege_renders_forbidden() {
        assert!(matches!(
            map_context_write_err(substrate_err("42501")),
            ApiError::Forbidden
        ));
    }

    /// Everything else stays a genuine internal error — the arms are additive, not a catch-all.
    #[test]
    fn any_other_sqlstate_stays_internal() {
        assert!(matches!(
            map_context_write_err(substrate_err("23503")),
            ApiError::Internal(_)
        ));
    }

    /// Whitespace only. `Café` keeps its accent — the fold that a *slug* may apply, a *name* may
    /// not.
    #[test]
    fn canonical_name_normalizes_whitespace_and_nothing_else() {
        assert_eq!(canonical_name("  Temper   KB \n"), "Temper KB");
        assert_eq!(canonical_name("Café  Notes"), "Café Notes");
        assert_eq!(canonical_name("   "), "");
        assert_eq!(canonical_name("!!!"), "!!!");
    }
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

    // ── Context rename ───────────────────────────────────────────────────────
    //
    // A rename moves the `(name, slug)` identity pair in place under an UNCHANGED owner. That is
    // the difference from `reassign` that every trap below descends from: the row can match itself.

    /// A personal context whose stored `name` differs from its `slug` — `mk_personal_context` sets
    /// both to the same string, which cannot express the legacy-repair or name-only cases.
    async fn mk_named_personal_context(
        pool: &PgPool,
        slug: &str,
        name: &str,
        owner: ProfileId,
    ) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
             VALUES ($1, $2, 'kb_profiles', $3) RETURNING id",
        )
        .bind(slug)
        .bind(name)
        .bind(*owner)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The stored identity pair, read straight from the row — the projection, not the outcome.
    async fn name_slug_of(pool: &PgPool, ctx: Uuid) -> (String, String) {
        let row = sqlx::query!("SELECT name, slug FROM kb_contexts WHERE id = $1", ctx)
            .fetch_one(pool)
            .await
            .unwrap();
        (row.name, row.slug)
    }

    /// How many `context_renamed` events this context carries. The trail is the ONLY attributability
    /// surface a rename has — `kb_contexts` keeps no before-image and no `updated` column.
    async fn rename_events(pool: &PgPool, ctx: Uuid) -> i64 {
        sqlx::query_scalar!(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
             WHERE t.name = 'context_renamed' AND (e.payload->>'context_id')::uuid = $1",
            ctx,
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .unwrap()
    }

    async fn is_readable(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
        sqlx::query_scalar!(
            r#"SELECT EXISTS(
                 SELECT 1 FROM resources_visible_to($1) WHERE resource_id = $2) AS "e!""#,
            *profile,
            resource,
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The empty-slug `400`, and **both** of the ways to reach it — the spec's "one `400` check, not
    /// two": an all-whitespace name canonicalizes to empty, a punctuation-only name sluggifies to
    /// empty. Neither may fall through to `create`'s `"context"` fallback, which would silently
    /// re-address the context to a slug the caller never asked for.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_name_with_no_addressable_content_is_refused(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let ctx = mk_named_personal_context(&pool, "notes", "Notes", alice).await;

        for bad in ["   ", "!!!"] {
            match rename(&pool, alice, ctx, bad).await {
                Err(ApiError::BadRequest(_)) => {}
                other => panic!("{bad:?} must be refused with 400, got {other:?}"),
            }
        }
        assert_eq!(
            name_slug_of(&pool, ctx).await,
            ("Notes".to_string(), "notes".to_string()),
            "a refused rename writes nothing"
        );
        assert_eq!(rename_events(&pool, ctx).await, 0);
    }

    /// The no-op arm: the canonical name already equals the stored one, so nothing is written and
    /// **no event is emitted**. The input differs from the stored name only in whitespace, which is
    /// precisely what canonicalization erases.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_canonically_identical_name_is_a_no_op_and_emits_nothing(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let ctx = mk_named_personal_context(&pool, "temper-kb", "Temper KB", alice).await;

        let outcome = rename(&pool, alice, ctx, "  Temper   KB  ").await.unwrap();
        assert!(!outcome.renamed);
        assert_eq!(outcome.name, "Temper KB");
        assert_eq!(outcome.slug, "temper-kb");
        assert_eq!(outcome.context_ref, "@alice/temper-kb");
        assert_eq!(rename_events(&pool, ctx).await, 0, "no-op emits no event");
    }

    /// The `409`, and the divergence from `create` it encodes: rename does not auto-suffix, so the
    /// caller is told the address is taken rather than handed `notes-2` without being asked.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_slug_taken_by_another_context_under_the_same_owner_conflicts(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        mk_named_personal_context(&pool, "notes", "Notes", alice).await;
        let scratch = mk_named_personal_context(&pool, "scratch", "Scratch", alice).await;

        match rename(&pool, alice, scratch, "Notes").await.unwrap_err() {
            ApiError::Conflict(msg) => assert!(
                msg.contains("notes"),
                "the refusal names the colliding slug: {msg}"
            ),
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(
            name_slug_of(&pool, scratch).await,
            ("Scratch".to_string(), "scratch".to_string()),
            "a refused rename writes nothing"
        );
        assert_eq!(rename_events(&pool, scratch).await, 0);
    }

    /// The happy path: **both** columns move, the event lands, and the outcome carries the composed
    /// new ref — the address the caller must use from now on, since the old one no longer resolves.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rename_moves_both_name_and_slug_and_emits(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let ctx = mk_named_personal_context(&pool, "proj", "Proj", alice).await;

        let outcome = rename(&pool, alice, ctx, "  Temper   KB ").await.unwrap();
        assert!(outcome.renamed);
        assert_eq!(outcome.name, "Temper KB", "stored canonical, not verbatim");
        assert_eq!(outcome.slug, "temper-kb");
        assert_eq!(outcome.owner_ref, "@alice");
        assert_eq!(
            outcome.context_ref, "@alice/temper-kb",
            "one sigil, not two — composed from the already-decorated owner_ref"
        );
        assert_eq!(
            name_slug_of(&pool, ctx).await,
            ("Temper KB".to_string(), "temper-kb".to_string())
        );
        assert_eq!(rename_events(&pool, ctx).await, 1);
    }

    /// **The self-exclusion regression test.** A rename whose slug does not move is still a rename.
    /// `reassign`'s collision query has no `AND id <> …` and correctly needs none; copied verbatim
    /// here it would find this context's own row and 409 against its own slug.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_name_only_rename_is_still_a_rename_and_does_not_collide_with_itself(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        // "Temper KB" and "Temper-KB" are different names that sluggify identically.
        let ctx = mk_named_personal_context(&pool, "temper-kb", "Temper KB", alice).await;

        let outcome = rename(&pool, alice, ctx, "Temper-KB").await.unwrap();
        assert!(outcome.renamed, "a display-name change is not a no-op");
        assert_eq!(outcome.slug, "temper-kb", "the address stayed put");
        assert_eq!(
            name_slug_of(&pool, ctx).await,
            ("Temper-KB".to_string(), "temper-kb".to_string())
        );
        assert_eq!(
            rename_events(&pool, ctx).await,
            1,
            "it emits like any rename"
        );
    }

    /// **The legacy-repair case**, and the reason the no-op arm compares the NAME. This context's
    /// stored name predates canonicalization; its own repaired form sluggifies identically, so a
    /// slug-comparison no-op arm would decline the one rename that fixes it — permanently.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_non_canonical_stored_name_can_be_repaired_to_its_canonical_form(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let ctx = mk_named_personal_context(&pool, "temper-kb", "Temper  KB", alice).await;

        let outcome = rename(&pool, alice, ctx, "Temper KB").await.unwrap();
        assert!(outcome.renamed, "the repair is a real state change");
        assert_eq!(
            name_slug_of(&pool, ctx).await,
            ("Temper KB".to_string(), "temper-kb".to_string())
        );
    }

    /// `canonical_name`'s **second** caller: a create stores the canonical name too. Without this
    /// the invariant would be honoured by one of two write paths, which is not an invariant.
    #[sqlx::test(migrations = "../../migrations")]
    async fn create_stores_a_canonical_name(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;

        let row = create(&pool, "kb_profiles", *alice, "  Temper   KB \n")
            .await
            .unwrap();
        assert_eq!(row.name, "Temper KB");
        assert_eq!(row.slug, "temper-kb");
        assert_eq!(
            name_slug_of(&pool, *row.id).await,
            ("Temper KB".to_string(), "temper-kb".to_string())
        );
    }

    /// The gate is wired, and it answers in **two** dialects: a plain member of the owning team can
    /// read the context but does not `can_manage` it → `403`; a stranger cannot see it at all →
    /// the incumbent `404`, byte-identical to `get_visible`'s refusal.
    #[sqlx::test(migrations = "../../migrations")]
    async fn rename_refuses_a_reader_with_403_and_a_stranger_with_404(pool: PgPool) {
        let member = mk_profile_ent(&pool, "member").await;
        let stranger = mk_profile_ent(&pool, "stranger").await;
        let acme = mk_team(&pool, "acme").await;
        add_member(&pool, acme, member, "member").await;
        let ctx = mk_team_context(&pool, "eng-notes", acme).await;

        assert!(matches!(
            rename(&pool, member, ctx, "Engineering Notes")
                .await
                .unwrap_err(),
            ApiError::Forbidden
        ));
        match rename(&pool, stranger, ctx, "Engineering Notes")
            .await
            .unwrap_err()
        {
            ApiError::NotFound(msg) => assert_eq!(msg, CONTEXT_REFUSAL),
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert_eq!(rename_events(&pool, ctx).await, 0);
    }

    /// The RBAC gate is an INVARIANT of `context_rename`, not merely a service pre-check — the
    /// TOCTOU backstop. Calling the function DIRECTLY with an emitter belonging to a profile that
    /// does not administer the context must still be rejected, atomically: `42501`, row untouched.
    /// (Model: `sql_guard_rejects_unauthorized_emitter_directly` above.)
    #[sqlx::test(migrations = "../../migrations")]
    async fn sql_guard_rejects_unauthorized_rename_emitter_directly(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let mallory = mk_profile_ent(&pool, "mallory").await;
        let ctx = mk_named_personal_context(&pool, "proj", "Proj", alice).await;

        let payload = temper_substrate::payloads::ContextRenamed {
            context_id: ContextId::from(ctx),
            from_name: "Proj".to_string(),
            from_slug: "proj".to_string(),
            to_name: "Hijacked".to_string(),
            to_slug: "hijacked".to_string(),
        };
        let payload_json = serde_json::to_value(&payload).unwrap();
        let emitter = emitter_of(&pool, mallory, "mallory").await;

        let res = sqlx::query_scalar::<_, Uuid>("SELECT context_rename($1,$2,$3,$4,$5)")
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
        assert_eq!(
            name_slug_of(&pool, ctx).await,
            ("Proj".to_string(), "proj".to_string()),
            "identity pair unchanged when the SQL guard rejects"
        );
    }

    /// The system-admin bypass holds at the SQL layer too. The subject is a context the admin
    /// **does not own** — the arm that admits an actor who cannot even *read* the context, and the
    /// reason the Rust gate probes `is_system_admin` above visibility. An admin-owned subject would
    /// have passed through the ownership branch and proved nothing about the bypass.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sql_guard_allows_system_admin_rename_directly(pool: PgPool) {
        let (admin, _member, _gating, _own_ctx) = seed_admin_team_context(&pool).await;
        let alice = mk_profile_ent(&pool, "alice").await;
        let ctx = mk_named_personal_context(&pool, "private", "Private", alice).await;
        assert!(
            !context_visible(&pool, *admin, ctx).await.unwrap(),
            "fixture precondition: the admin must NOT be able to read this context"
        );

        let payload = temper_substrate::payloads::ContextRenamed {
            context_id: ContextId::from(ctx),
            from_name: "Private".to_string(),
            from_slug: "private".to_string(),
            to_name: "Renamed By Admin".to_string(),
            to_slug: "renamed-by-admin".to_string(),
        };
        let payload_json = serde_json::to_value(&payload).unwrap();
        let emitter = emitter_of(&pool, admin, "admin").await;

        sqlx::query_scalar::<_, Uuid>("SELECT context_rename($1,$2,$3,$4,$5)")
            .bind(&payload_json)
            .bind(emitter)
            .bind(serde_json::json!({}))
            .bind(None::<Uuid>)
            .bind(None::<Uuid>)
            .fetch_one(&pool)
            .await
            .expect("system admin rename through the SQL function");

        assert_eq!(
            name_slug_of(&pool, ctx).await,
            (
                "Renamed By Admin".to_string(),
                "renamed-by-admin".to_string()
            )
        );
    }

    /// `a-context-never-loses-its-contents-to-a-rename`. Spec §"What a rename does not touch" argues
    /// this holds *because nothing is keyed by slug* — resources are homed on `(anchor_table,
    /// anchor_id)` where `anchor_id` is a UUID. The argument is sound and it is still an argument;
    /// this makes it falsifiable.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rename_does_not_disturb_the_contexts_contents(pool: PgPool) {
        let alice = mk_profile_ent(&pool, "alice").await;
        let stranger = mk_profile_ent(&pool, "stranger").await;
        let ctx = mk_named_personal_context(&pool, "proj", "Proj", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        let before = sqlx::query!(
            r#"SELECT anchor_table AS "t!", anchor_id AS "i!", owner_profile_id
                 FROM kb_resource_homes WHERE resource_id = $1"#,
            r,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(is_readable(&pool, alice, r).await);
        assert!(!is_readable(&pool, stranger, r).await);

        let outcome = rename(&pool, alice, ctx, "Temper KB").await.unwrap();
        assert!(outcome.renamed);

        let after = sqlx::query!(
            r#"SELECT anchor_table AS "t!", anchor_id AS "i!", owner_profile_id
                 FROM kb_resource_homes WHERE resource_id = $1"#,
            r,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((after.t, after.i), (before.t, before.i), "home row unmoved");
        assert_eq!(after.owner_profile_id, before.owner_profile_id);
        assert!(is_readable(&pool, alice, r).await, "still readable");
        assert!(
            !is_readable(&pool, stranger, r).await,
            "and still not readable by anyone new"
        );
        assert!(can_modify(&pool, alice, r).await, "authorship unchanged");
    }
}
