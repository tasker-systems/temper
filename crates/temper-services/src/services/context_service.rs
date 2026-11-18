//! Context CRUD service over the substrate.
//!
//! Visibility is centralized in the `context_visible_to(principal, context)`
//! SQL function. It was born at `20260627000001`, but its live body is
//! `20260712000010:147-151`, where it became a one-line delegate to
//! `context_readable_by_profile` (`:135-140`) over `contexts_readable_by`, whose
//! authoritative four arms now live at `20260807000010:103-129`: own personal context,
//! OR a context owned by an enclosing team, OR a context shared (`kb_team_contexts`) to
//! an enclosing team, OR an explicit `kb_access_grants` read-grant on the context
//! (profile-anchored, or team-anchored on a reachable team). "Enclosing" is the ancestor
//! CLOSURE, not direct membership: `profile_reachable_teams` (`20260804000010:117-127`)
//! expands each membership UP the enclosure chain, so a context held by an ancestor team
//! reads for everyone beneath it. Every read/resolve site
//! below calls that one predicate so they cannot drift. `kb_owner_table`/`kb_owner_id`/
//! `updated` are synthesized to the `ContextRow` shape from the substrate's
//! `owner_table`/`owner_id`/`created` columns. Resource counts come from
//! `kb_resource_homes`, intersected with the caller's own `resources_visible_to` and the
//! soft-delete floor — the anchor gate says who gets an answer, not what is in it.
//! Context creation is a plain INSERT (no event emission —
//! product decision 5: contexts are infrastructure).

use sqlx::PgPool;

use crate::auth::{Caller, HumanPrincipal};
use crate::authz::{ContextAdminAuthority, Principal, TwoSidedAuthority, TwoSidedScope};
use crate::error::{ApiError, ApiResult};
use crate::services::team_service;
use temper_core::context_ref::{ContextOwnerRef, ContextRef};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::reach::Reach;
use temper_core::types::team::TeamRole;
use temper_workflow::operations::sluggify;

pub use temper_core::types::context::{
    ContextActivity, ContextCreateRequest, ContextRow, ContextRowWithCounts, InheritedPublicWithin,
    InheritedReadGrant, InheritedShare, ReassignContextOutcome, ReassignContextRequest,
    RenameContextOutcome, RenameContextRequest, RestoreContextOutcome, RetireContextOutcome,
    ShareContextOutcome, ShareContextRequest, UnshareContextOutcome,
};

/// When the profile last changed something in each context they can read — one row per context
/// they ever changed something in, newest first. `GET /api/me/context-activity`.
///
/// Read through `contexts_changed_by` (20261107000000), which aggregates the profile's own authored
/// changes per context and then gates once by `contexts_readable_by`; a context the profile cannot
/// read, or a retired one, has no row. A read of its own, not a column of [`list_visible`], because
/// it scans the profile's whole history and the contexts list is read on every page load.
pub async fn activity_of(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Vec<ContextActivity>> {
    let rows = sqlx::query_as!(
        ContextActivity,
        r#"
        SELECT a.context_id AS "context_id!: ContextId",
               a.changed_at AS "last_changed_at!"
          FROM contexts_changed_by($1) a
         ORDER BY a.changed_at DESC, a.context_id
        "#,
        *profile_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List all contexts visible to the profile (owned + team-shared), with resource counts.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Vec<ContextRowWithCounts>> {
    // ── Why `resource_count` carries two joins ─────────────────────────────────────
    //
    // A returned count is a disclosure about every row that can move it. The anchor gate
    // below decides whether the caller gets an answer at all; the caller's own read
    // predicate has to decide which homes are IN that answer. `kb_resource_homes` carries
    // neither `is_active` nor a reader column (20260624000001:276-284), so a bare COUNT
    // over it counts soft-deleted resources and resources the caller cannot read —
    // `context_visible_to` gates the CONTEXT, and says nothing about its contents.
    // These two joins are the pair `graph_home_contexts` already applies to this same
    // field on this same anchor (20260712000010:327-331), so the two doors onto a context
    // cannot disagree about which resources exist. But on this anchor the pair is today
    // CO-EXTENSIVE, and neither join is the one that "does the work".
    // `resources_visible_to` ENDS with its own soft-delete floor — `JOIN kb_resources r
    // ON r.id = v.resource_id AND r.is_active` (20260807000010:223-225) — so it can
    // never admit a dead resource; and in the other direction its context arm admits
    // every home in a context `contexts_readable_by` admits (20260807000010:199-205),
    // which is the same set the WHERE below gates the row on
    // (20260712000010:147-151 → :135-140 → 20260807000010:143-153), so every LIVE home
    // reachable here is already inside it. Drop either join and the number is unchanged,
    // which is why context_resource_count_read_gate_test.rs pins the property and
    // isolates NEITHER predicate. Both stay anyway: mutual backstops if either function
    // narrows, and symmetry with the sibling — not two independent filters. What is
    // genuinely beyond witness is a LIVE resource in a readable context that the caller
    // cannot read, not constructible while those two functions agree; the reader
    // predicate is written down regardless, so the count is correct BECAUSE of the
    // caller rather than by a coincidence between two functions that may yet narrow.
    // Deliberately no `::int`: that sibling casts because its `RETURNS TABLE` declares
    // `integer`; here the column is `ContextRowWithCounts.resource_count: i64`, which
    // `count(*)`'s bigint already is.
    // A correlated scalar subquery rather than a join, so the statement stays one row per
    // context with no GROUP BY to hold in step with the select list.
    //
    //
    // ── Why the gate and the count read two sets computed once ─────────────────────
    //
    // The row gate is `contexts_readable_by($1)`, the body `context_visible_to` delegates to
    // (20260712000010:135-151), so the rows are exactly the contexts `context_visible_to`
    // admits and the two doors onto a context cannot disagree. It is evaluated ONCE per call,
    // as a MATERIALIZED set, and so is `resources_visible_to($1)` for the count: called per
    // context, each call re-ran the reader's whole read set, the public_within arm with its
    // fence and per-row effect walks included, once for every context in the table. Same
    // functions, same rows; one evaluation each instead of one per context.
    //
    // Kept out of the SQL literal on purpose: sqlx keys the .sqlx cache on the whole query
    // TEXT, comments included, so a prose correction inside the literal orphans the cache
    // entry and forces a regenerate. Reasoning about the query as a whole belongs here,
    // where correcting it is free; only what pins a single line stays in the SQL.
    let rows = sqlx::query_as!(
        ContextRowWithCounts,
        r#"
        WITH readable AS MATERIALIZED (
                 SELECT r.context_id FROM contexts_readable_by($1) r),
             visible AS MATERIALIZED (
                 SELECT v.resource_id FROM resources_visible_to($1) v)
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
               -- Computed INSIDE the visibility-gated query (the WHERE below), so a context the
               -- caller cannot read is absent rather than present-and-false.
               context_authorable_by_profile($1, c.id) AS "can_write!",
               -- Always `false` here: this is the read axis, and a retired context is invisible
               -- to `context_visible_to` by construction, so this WHERE can never admit one. The
               -- column stays in the select list anyway — one honest row shape for both doors.
               NOT c.is_active AS "retired!",
               -- Counted through the caller's own read predicate, not off the home rows —
               -- the two joins are co-extensive on this anchor and neither is redundant by
               -- accident. The reasoning, and what the witness does NOT isolate, is above.
               (SELECT count(*)
                  FROM kb_resource_homes rh
                  JOIN visible v ON v.resource_id = rh.resource_id
                  JOIN kb_resources r ON r.id = rh.resource_id AND r.is_active
                 WHERE rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id) AS "resource_count!"
          FROM kb_contexts c
         WHERE c.id IN (SELECT rc.context_id FROM readable rc)
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
               END AS "owner_ref!",
               -- Inside the same visibility gate as the WHERE below: an unreadable context is the
               -- one shared refusal, never a row saying `can_write: false`.
               context_authorable_by_profile($1, c.id) AS "can_write!",
               -- Always `false` here: the read axis never admits a retired context (see the same
               -- note on `list_visible`). Kept in the select list for one honest row shape.
               NOT c.is_active AS "retired!",
               -- In the SELECT list, so it runs only for the row the WHERE below admitted: the
               -- reach functions take "the reader reads it" as their precondition.
               context_reached_via($1, c.id) AS "reach: Reach"
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

/// The refusal for restoring a context the person-erasure act erased (migration
/// `20261022100000`, R7). A 410, not a 404: the caller administers the context (the gate has
/// already passed), so its existence is no secret from them, and it is gone for good.
const CONTEXT_ERASED: &str = "context was erased with its owner and cannot be restored";

/// The refusal the two **self-namespace** arms render — `@me/slug` and `+team/slug`, where the
/// caller either owns the namespace or is a member of it, so echoing the slug they supplied
/// discloses nothing. [`CONTEXT_REFUSAL`]'s doc records that exemption for `@me`.
///
/// It exists because each of those arms now has **two** ways to fail — no such slug, and a slug
/// whose context is retired — and they must be indistinguishable. Rendering one of them bare
/// would let a member tell "never existed" from "retired", which is the one-bit existence oracle
/// the `@handle` arm's three-way byte-identity exists to prevent. One function so the two arms
/// cannot drift apart, for the reason the constant above gives.
fn context_slug_refusal(slug: &str) -> ApiError {
    ApiError::NotFound(format!("context {slug} not found or not readable"))
}

/// Resolve a context ref to a `ContextId`, gated to what `principal` may see.
///
/// The single source of truth for context resolution. `@me` uses the caller's
/// profile; `@handle`/`+team` resolve the owner then the `(owner, slug)` row;
/// a bare UUID names the row directly.
///
/// **All four arms end at `context_visible_to`.** Resolving the row is never the gate — each arm
/// finds a `(owner, slug)` or an id and then asks the read predicate, so this function and
/// `context_visible_to` cannot disagree. That matters since context retirement gave the predicate
/// something to refuse for a row that is still present: a retired context resolves through no arm,
/// including `@me` for its own owner and `+team` for a member. Retired contexts are addressed on
/// the ADMIN axis (`get_retired_administered`, `resolve_context_id_for_restore`), never here.
///
/// **`+team/slug` answers by reading, not by membership.** A direct member resolves as always. A
/// caller with no membership row on the team resolves a context the team owns only when it is in
/// the caller's reading set: a member of a descendant team, a person who reads the team through
/// public_within, or a holder of a share or a read grant on that context. Resolving is a read: no
/// write behind a resolved id is admitted without the backend's own authorship check.
///
/// Error taxonomy:
/// - `Id`/`Handle`/profile-context miss → `NotFound`
/// - Team non-member, for any context the caller does not read → `Forbidden`, one answer whether
///   the slug is real, missing, retired or unreadable (existence of team contexts not leaked)
/// - Soft-deleted team → `NotFound`, byte-identical to a team that never existed (a soft-deleted
///   team is not a team you are being refused; it is not a team)
/// - Retired context → `NotFound`, byte-identical to a slug that never existed **within the same
///   arm** (plain backticks, not an intra-doc link: `context_slug_refusal` is private and this
///   item is public, which `cargo doc` refuses)
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
            // Gated like every other arm. `lookup_profile_context` reads `kb_contexts` by
            // `(owner_id, slug)` and knows nothing about visibility, so on its own this arm
            // resolves a context `context_visible_to` refuses — which since context retirement
            // includes the caller's OWN retired contexts (verified: after retirement
            // `context_visible_to(owner, ctx)` is false while the row is still there under its
            // mangled slug). Nothing is stranded by refusing: `resources_visible_to`'s owner arm
            // has no context floor, so the owner still reaches their own resources through an
            // unscoped read — what they lose is using a retired ref as a SCOPE, which is what
            // "addressed on the admin axis, never the read axis" means.
            ContextOwnerRef::Me => {
                let cid = lookup_profile_context(pool, *principal, slug).await?;
                // Refused through `context_slug_refusal`, not bare: `lookup_profile_context`'s own
                // miss names the slug, so a bare refusal here would be a second, distinguishable
                // answer on one arm.
                ensure_context_visible(pool, *principal, *cid)
                    .await
                    .map_err(|e| match e {
                        ApiError::NotFound(_) => context_slug_refusal(slug),
                        other => other,
                    })?;
                Ok(cid)
            }
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
                // `AND t.is_active` is the invariant `20260703000001_team_metadata_soft_delete.sql`
                // declares at chokepoint 1: *"a soft-deleted team is not an effective membership …
                // membership in a soft-deleted team confers nothing anywhere."* Every SQL read path
                // honours it (`profile_effective_teams` → `profile_reachable_teams` →
                // `contexts_readable_by` / `resources_visible_to`); this arm resolves the team in
                // Rust and so has to state it itself, or a member of a dead team could still use
                // `+that-team/ctx` as a scope — which is exactly the membership conferring
                // something.
                //
                // A soft-deleted team refuses as `NotFound`, byte-identical to a team that never
                // existed, and deliberately NOT as the `Forbidden` below: `Forbidden` says "this
                // team exists and you are not in it", which both discloses its existence and is
                // false — the caller IS in it. There is no team here to be refused access to.
                let team_id = sqlx::query_scalar!(
                    "SELECT id FROM kb_teams WHERE slug = $1 AND is_active",
                    team_slug
                )
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| {
                    ApiError::NotFound(format!("team {team_slug} not found or not readable"))
                })?;
                // Membership decides which branch answers, not whether there is an answer.
                //
                // No `is_active` filter here, and that is not an omission: an inactive team never
                // reaches this line, so the only rows this can match already belong to an active
                // team. `profile_effective_teams`'s own filter lives on the team side for the same
                // reason. Adding a second copy would state the invariant in a place that cannot be
                // reached to violate it, and imply the lookup above were optional.
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
                    return resolve_team_context_for_non_member(pool, principal, team_id, slug)
                        .await;
                }
                let id = sqlx::query_scalar!(
                    "SELECT id FROM kb_contexts \
                     WHERE owner_table = 'kb_teams' AND owner_id = $1 AND slug = $2",
                    team_id,
                    slug
                )
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| context_slug_refusal(slug))?;
                // Membership is not visibility. The check above admits ANY role — `watcher`
                // included — and this lookup reads `kb_contexts` directly, so without this the arm
                // resolves a retired team context for a member who never administered it. Every
                // caller documents this function as visibility-gated (`substrate_read`,
                // `handlers::ingest`, `handlers::graph`, `tools::cognitive_maps`), and the two
                // sibling arms already end here; this one did not.
                // Same refusal as the miss directly above -- a member at ANY role, `watcher`
                // included, must not be able to tell "no such slug" from "retired".
                ensure_context_visible(pool, *principal, id)
                    .await
                    .map_err(|e| match e {
                        ApiError::NotFound(_) => context_slug_refusal(slug),
                        other => other,
                    })?;
                Ok(ContextId::from(id))
            }
        },
    }
}

/// The `+team/slug` arm for a caller with no direct membership row on the team: `Ok` only for a
/// context the team owns, by that slug, that the caller reads. Every other case is the one bare
/// [`ApiError::Forbidden`] a non-member has always received.
///
/// **Who gets here and resolves.** Anyone whose reading set (`contexts_readable_by`) holds a
/// context the team owns: a member of a descendant team (the team closure runs self-or-ancestor,
/// so a child's member reads its parent's owned contexts by membership), a person who reads the
/// team through an effective public_within row, and a holder of a share or a read grant on that
/// context. A machine principal's reading set never includes the public_within arm (the fence
/// inside `contexts_public_within`), so it resolves here only through membership-shaped reach.
///
/// **No existence oracle.** A non-member who does not read the context must learn nothing about
/// the slug: missing, retired and unreadable are one `Forbidden`, the answer a non-member received
/// before this branch existed. The slug never reaches SQL: the query reads the caller's readable
/// contexts of this team as one set, joined once against the caller's reading set, and the slug is
/// matched in Rust. So the database work depends only on the caller and the team, which the caller
/// named and which the arm's 404/403 split already discloses, never on whether a context by that
/// slug exists. Retired contexts are absent from the reading set, so a retired slug is a miss here.
///
/// **No slug refusal.** [`context_slug_refusal`] echoes the slug and says "not found"; it is the
/// member-only answer, because only a member is entitled to learn that the team has no live
/// context by that name. A reader who is not a member learns it only by reading the team's
/// contexts, through the reads that already show them.
///
/// **Resolving grants no act.** Every caller that writes behind a resolved id (create, ingest,
/// move) re-checks authorship in the backend, so a reader who resolves here and then targets the
/// context with a write gets the backend's "can read but cannot author" `403` and nothing is
/// written.
async fn resolve_team_context_for_non_member(
    pool: &PgPool,
    principal: ProfileId,
    team_id: uuid::Uuid,
    slug: &str,
) -> ApiResult<ContextId> {
    let readable = sqlx::query!(
        r#"SELECT c.id, c.slug
             FROM contexts_readable_by($1) r
             JOIN kb_contexts c ON c.id = r.context_id
            WHERE c.owner_table = 'kb_teams' AND c.owner_id = $2"#,
        *principal,
        team_id
    )
    .fetch_all(pool)
    .await?;
    readable
        .into_iter()
        .find(|row| row.slug == slug)
        .map(|row| ContextId::from(row.id))
        .ok_or(ApiError::Forbidden)
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
///
/// **No self-exclusion, and that is a dependency rather than an oversight.** [`rename`] carries an
/// `AND id <> $4` its own doc calls load-bearing; this has none, so a caller whose row already
/// holds the candidate slug would be told its own address is taken and moved off it. That is
/// unreachable today by exactly one argument: no caller ever asks this for a slug its own row
/// already holds. [`create`] has no row yet; [`retire`] asks for `<slug>-retired` while the row
/// holds `<slug>`; [`restore`] asks for `sluggify(name)` while the row holds the mangled form.
///
/// The third of those held only after `20260826000130` floored [`rename`] on `is_active`. Before
/// it, a retired row could be renamed back to its name's canonical slug, and `restore` would then
/// find its own address taken, relocate to `<slug>-2`, and report `slug_changed` for a restore that
/// should not have moved. **A path that moves a retired row's slug to its canonical form re-opens
/// this**, so the two live together: widen what may rename a retired context and this needs the
/// exclusion.
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
/// - `Team(slug)` → the team (must exist **and be active**, else `NotFound`); the
///   caller must be `owner`/`maintainer` on it (reuses `team_service`'s role check
///   — no duplicated authz), else `Forbidden`.
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
            // `AND is_active` is the invariant `20260703000001_team_metadata_soft_delete.sql`
            // declares in its own header: *"membership in a soft-deleted team confers nothing
            // anywhere."* This is the write-side twin of the read-side arm in
            // [`resolve_context_ref`], which carries the full argument. Without it an
            // owner/maintainer of a dead team can mint a context OWNED by it — state the read
            // paths then refuse to show anyone, this one included.
            //
            // `NotFound`, not the `Forbidden` below, and byte-identical to a team that never
            // existed: `Forbidden` would disclose that the slug names something and would be false,
            // since the caller genuinely is a member. There is no team here to be refused.
            let team_id = sqlx::query_scalar!(
                "SELECT id FROM kb_teams WHERE slug = $1 AND is_active",
                slug
            )
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("team {slug} not found or not readable")))?;
            // An inactive team cannot reach this line (the lookup above), and `role_on_team`
            // answers for live teams only (see its doc).
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
    profile_id: ProfileId,
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
                  END AS "owner_ref!",
                  -- Placeholder: the real value is computed in a SEPARATE statement below.
                  -- `context_authorable_by_profile` is STABLE, so called here it would read the
                  -- snapshot from BEFORE this INSERT and could not see the row being created —
                  -- the personal-owned arm would answer `false` for the creator's own context.
                  false AS "can_write!",
                  -- A freshly created context is never retired.
                  false AS "retired!",
                  -- Reach is answered on the get only.
                  NULL::text AS "reach: Reach"
        "#,
        *id,
        owner_table,
        owner_id,
        slug,
        name
    )
    .fetch_one(pool)
    .await?;

    // Authority, in its own statement so it sees the committed row. Computed rather than assumed
    // true: a team-owned context is created under a role gate that is not the same predicate as
    // `context_authorable_by_profile`, so the creator of one is not guaranteed to author it, and a
    // hardcoded `true` would make the one response that can disagree with the write gate the one
    // response nobody checks.
    let can_write: bool = sqlx::query_scalar!(
        r#"SELECT context_authorable_by_profile($1, $2) AS "can_write!""#,
        *profile_id,
        *id,
    )
    .fetch_one(pool)
    .await?;

    Ok(ContextRow { can_write, ..row })
}

/// Assert both `context_id` and a **live** `team_id` exist before a `kb_team_contexts` write —
/// otherwise a nonexistent id hits the FK constraint and surfaces as an opaque 500
/// instead of a clean 404. A soft-deleted team is a 404 too, admins included: nothing is written
/// onto a dead team (soft-delete design D4). Called AFTER the admin gate (auth stays first) and BEFORE
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
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1 AND is_active) AS "ok!""#,
        team_id
    )
    .fetch_one(pool)
    .await?;
    if !team_exists {
        return Err(ApiError::NotFound("team not found".to_string()));
    }
    Ok(())
}

/// Whether a context is someone else's personal space: owned by a profile other than `caller`, or by
/// a personal team that is not `caller`'s. A system admin's arm does not reach it — only its owner
/// acts on it, and erasure (task `01a191b4`). The SQL `context_personal_owner` is the one
/// definition of whose personal space a context is; this calls it.
pub(crate) async fn is_others_personal_context(
    pool: &PgPool,
    context_id: uuid::Uuid,
    caller: ProfileId,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT COALESCE(context_personal_owner($1) <> $2, false) AS "p!: bool""#,
        context_id,
        *caller,
    )
    .fetch_one(pool)
    .await?)
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

/// The manage-capable roles, derived from `can_manage` rather than restated. This is the
/// admin axis's team half, and it must stay pinned to the one Rust definition of "manage":
/// a role added to `TeamRole` and forgotten here would silently widen or narrow who can see
/// retired contexts.
///
/// No `TeamRole::iter()` exists (no `strum` in this workspace — checked; `Display`/`ToString`
/// aren't derived either, so the stored spelling is spelled out below rather than borrowed from
/// a trait that doesn't exist). The explicit four-variant array is exhaustive today but is NOT
/// compiler-checked for it; if a fifth role is ever added, this is a call site that must be
/// remembered, same as every other explicit `[TeamRole::A, TeamRole::B, ...]` in this crate.
fn manage_capable_roles() -> Vec<String> {
    [
        TeamRole::Owner,
        TeamRole::Maintainer,
        TeamRole::Member,
        TeamRole::Watcher,
    ]
    .into_iter()
    .filter(|r| team_service::can_manage(*r))
    .map(|r| {
        // The spelling `kb_team_members.role` stores and every write path binds — see
        // `team_service::create_team`'s `'owner'` literal and `add_member`'s `$3`
        // (bound as `TeamRole` directly, via `#[sqlx(type_name = "team_role", rename_all =
        // "snake_case")]`). Written out because `TeamRole` derives `Serialize` (JSON-facing,
        // not SQL-facing) and nothing else that stringifies it.
        match r {
            TeamRole::Owner => "owner",
            TeamRole::Maintainer => "maintainer",
            TeamRole::Member => "member",
            TeamRole::Watcher => "watcher",
        }
        .to_string()
    })
    .collect()
}

/// List contexts retired that `profile_id` administers — the admin axis, never the read axis.
///
/// A retired context is invisible to `context_visible_to` by construction (`is_active` floors
/// every arm of `contexts_readable_by_teams`), so this is not `list_visible` with a flipped
/// flag: it is a **different gate** over the same row shape. Three arms admit:
///
/// 1. own the context (`kb_profiles` arm);
/// 2. hold a `manage_capable_roles` role on its owning team (`kb_teams` arm) — no `EXISTS`/`IN`
///    restates `can_manage`'s `Owner | Maintainer` split in SQL; the role list is a bound
///    parameter derived from that one Rust function, so a role added there and forgotten here is
///    structurally impossible to get out of sync (it just wouldn't be in the bound array);
/// 3. be a system admin (`access_service::is_system_admin`), for any context but a personal one —
///    a system admin sees that a personal team exists and its roster, nothing of its contexts
///    (task `01a191b4`).
///
/// **Why arm 3 belongs here.** This is an admin-axis listing — the same category as
/// `subscription_service::list`, which already ORs a system-admin bool into its own visibility
/// WHERE — not a visibility predicate. Visibility predicates (`resources_visible_to`,
/// `edges_visible_to`, `contexts_readable_by_teams`, `cogmaps_readable_by`) deliberately carry no
/// system-admin arm, and that separation is untouched by this change: an admin sees every
/// *retired* row through this door, but still sees only their own reach through the ordinary read
/// predicates. Computed in Rust and passed as a bound bool, the same shape
/// `subscription_service::list` uses — not `is_system_admin($1)` inlined into the SQL, which
/// would be a second spelling of the same rule.
///
/// `resource_count` carries `list_visible`'s subquery **verbatim**: it counts through the
/// caller's OWN read predicate (`resources_visible_to`), not the admin predicate. The count
/// answers "how much of this can the caller still read", and for a retired context that is a
/// genuinely mixed number — **not** always zero, which is the tempting and wrong summary:
///
/// * Resources the caller **owns** still count. `resources_visible_to`'s first arm is
///   `kb_resource_homes.owner_profile_id = p_profile` and carries no context floor, which is the
///   same anti-trap property that lets an owner move their work out of a retired context. Verified
///   against the live database: owner + own resource + retired context yields `1`, not `0`.
/// * Resources the caller reached only **through the container** drop to zero, because that arm of
///   `resources_visible_to` routes through `contexts_readable_by_teams`, which floors on
///   `is_active`.
///
/// So a system admin listing someone else's retired context sees `0` while its owner sees their own
/// resources still counted. That asymmetry is correct — it is the read predicate answering
/// truthfully per caller — and it is why the subquery must NOT be switched to the admin predicate.
/// The comment on `list_visible` gives the fuller argument for the subquery's shape.
pub async fn list_retired_administered(
    pool: &PgPool,
    caller: &Caller,
) -> ApiResult<Vec<ContextRowWithCounts>> {
    let profile_id = caller.profile_id();
    let roles = manage_capable_roles();
    let is_admin = Principal::from(caller).system_admin(pool).await?.is_some();
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
               -- Always `false`: `context_authorable_by_profile` floors on `c.is_active`
               -- (20260826000110), so a retired context is unwriteable by construction.
               context_authorable_by_profile($1, c.id) AS "can_write!",
               NOT c.is_active AS "retired!",
               -- Carried VERBATIM from `list_visible` — counted through the caller's OWN read
               -- predicate, which can never admit a resource homed in a retired context. See the
               -- doc comment above.
               (SELECT count(*)
                  FROM kb_resource_homes rh
                  JOIN resources_visible_to($1) v ON v.resource_id = rh.resource_id
                  JOIN kb_resources r ON r.id = rh.resource_id AND r.is_active
                 WHERE rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id) AS "resource_count!"
          FROM kb_contexts c
         WHERE NOT c.is_active
           AND ( ($3 AND COALESCE(context_personal_owner(c.id) = $1, true))
                 OR (c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                 OR (
                   c.owner_table = 'kb_teams'
                   AND EXISTS (
                     SELECT 1 FROM kb_team_members tm
                       JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                      WHERE tm.team_id = c.owner_id
                        AND tm.profile_id = $1
                        AND tm.role::text = ANY($2)
                   )
                 )
               )
         ORDER BY c.name
        "#,
        *profile_id,
        &roles,
        is_admin,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get one retired context by ID, scoped to the ADMIN axis — [`list_retired_administered`]'s
/// same predicate over a single id, so anything reachable in that listing is reachable here too
/// (an incoherent pair otherwise: a caller could see a row named but never inspect it). The same
/// three arms admit: own the context, hold a `manage_capable_roles` role on its owning team, or
/// be a system admin — see that function's doc for why the admin arm belongs on this axis and not
/// on a visibility predicate.
///
/// Refuses with the same `CONTEXT_REFUSAL` every other context lookup renders — a missing
/// context, a live one, and one this caller does not administer are indistinguishable, for the
/// reason that constant's doc gives.
pub async fn get_retired_administered(
    pool: &PgPool,
    caller: &Caller,
    context_id: ContextId,
) -> ApiResult<ContextRow> {
    let profile_id = caller.profile_id();
    let roles = manage_capable_roles();
    let is_admin = Principal::from(caller).system_admin(pool).await?.is_some();
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
               END AS "owner_ref!",
               context_authorable_by_profile($1, c.id) AS "can_write!",
               NOT c.is_active AS "retired!",
               -- The admin axis: the caller administers a context nobody reads, so no reach.
               NULL::text AS "reach: Reach"
          FROM kb_contexts c
         WHERE c.id = $2
           AND NOT c.is_active
           AND ( ($4 AND COALESCE(context_personal_owner(c.id) = $1, true))
                 OR (c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                 OR (
                   c.owner_table = 'kb_teams'
                   AND EXISTS (
                     SELECT 1 FROM kb_team_members tm
                       JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                      WHERE tm.team_id = c.owner_id
                        AND tm.profile_id = $1
                        AND tm.role::text = ANY($3)
                   )
                 )
               )
        "#,
        *profile_id,
        *context_id,
        &roles,
        is_admin,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(CONTEXT_REFUSAL.to_string()))
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
    authed: &HumanPrincipal,
    context_id: uuid::Uuid,
    req: &ShareContextRequest,
) -> ApiResult<ShareContextOutcome> {
    let scope = TwoSidedScope::context(context_id, req.team_id);
    let on_admin_arm = two_sided_admin_arm(pool, authed, scope).await?;
    ensure_context_and_team_exist(pool, context_id, req.team_id).await?;
    let mut tx = two_sided_write_tx(pool, authed, scope, on_admin_arm).await?;
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
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
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
    authed: &HumanPrincipal,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<UnshareContextOutcome> {
    let scope = TwoSidedScope::context(context_id, team_id);
    let on_admin_arm = two_sided_admin_arm(pool, authed, scope).await?;
    ensure_context_and_team_exist(pool, context_id, team_id).await?;
    let mut tx = two_sided_write_tx(pool, authed, scope, on_admin_arm).await?;
    let result = sqlx::query!(
        "DELETE FROM kb_team_contexts WHERE context_id = $1 AND team_id = $2",
        context_id,
        team_id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
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
    authed: &HumanPrincipal,
    context_id: uuid::Uuid,
    to_team_id: uuid::Uuid,
) -> ApiResult<ReassignContextOutcome> {
    let caller = ProfileId::from(authed.profile().id);
    let scope = TwoSidedScope::context(context_id, to_team_id);
    let on_admin_arm = two_sided_admin_arm(pool, authed, scope).await?;
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

    let inherited_public_within = public_within_reach_of(pool, context_id, to_team_id).await?;

    if cur.owner_table == "kb_teams" && cur.owner_id == to_team_id {
        return Ok(ReassignContextOutcome {
            context_id,
            owner_ref: team_owner_ref(pool, to_team_id).await?,
            reassigned: false,
            inherited_shares,
            inherited_read_grants,
            inherited_public_within,
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
    let mut tx = two_sided_write_tx(pool, authed, scope, on_admin_arm).await?;
    temper_substrate::writes::reassign_context_on(
        &mut tx,
        temper_substrate::ids::ContextId::from(context_id),
        (cur.owner_table.as_str(), cur.owner_id),
        ("kb_teams", to_team_id),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(map_context_write_err)?;
    tx.commit().await?;

    Ok(ReassignContextOutcome {
        context_id,
        owner_ref: team_owner_ref(pool, to_team_id).await?,
        reassigned: true,
        inherited_shares,
        inherited_read_grants,
        inherited_public_within,
    })
}

/// The `public_within` reach a context owned by `team_id` has: one entry per effective row with
/// the team as T, naming the anchor and counting its audience. Empty for a connection's home
/// context, which no row ever opens. The caller of a reassign manages the team (or is a system
/// admin), and the team's detail shows its rows to its members, so the anchors are named.
async fn public_within_reach_of(
    pool: &PgPool,
    context_id: uuid::Uuid,
    team_id: uuid::Uuid,
) -> ApiResult<Vec<InheritedPublicWithin>> {
    Ok(sqlx::query!(
        r#"SELECT a.id AS "anchor_id!", a.slug AS "slug!",
                  au.teams AS "teams!", au.principals AS "principals!"
             FROM kb_team_public_within pw
             JOIN kb_teams a ON a.id = pw.anchor_id
             CROSS JOIN LATERAL public_within_audience(pw.anchor_id) au
            WHERE pw.team_id = $1
              AND public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
              AND NOT EXISTS (SELECT 1 FROM kb_connections k WHERE k.home_context_id = $2)
            ORDER BY a.slug"#,
        team_id,
        context_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| InheritedPublicWithin {
        anchor_id: r.anchor_id,
        anchor_ref: format!("+{}", r.slug),
        teams: r.teams,
        principals: r.principals,
    })
    .collect())
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
/// reason `ScopedAuthority::denial_for` exists. A system admin on someone else's personal context
/// gets a third answer, a `403` that says so (task `01a1933d`). The `context_rename` SQL function re-runs the same
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
    authed: &Caller,
    context_id: uuid::Uuid,
    name: &str,
) -> ApiResult<RenameContextOutcome> {
    let caller = authed.profile_id();
    crate::authz::authorize::<ContextAdminAuthority>(pool, Principal::from(authed), context_id)
        .await?;

    // The current identity pair plus the already-sigil'd owner ref, in one read. A rename leaves
    // `(owner_table, owner_id)` untouched, so the ref composed from this row is still correct after
    // the write. The `CASE` is the incumbent both-kinds spelling (`list_visible`, `get_visible`,
    // `create`) — `team_owner_ref` is team-only and would `fetch_one`-panic on a profile-owned
    // context.
    //
    // `fetch_optional`, not `fetch_one`: the gate's `SystemAdmin` arm admits without consulting the
    // subject's existence (it probes the caller's governance grant), so a system admin naming a
    // context id that does not exist reaches here. That is the read refusal, not a 500.
    let cur = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!", slug, name, is_active,
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

    // 0. A retired context is not renameable. `context_rename` carries the same guard as an
    //    in-transaction invariant (`20260826000130`), and this is its advisory half — the same
    //    division the authorization gate uses: refuse here on the common path, let the SQL guard
    //    close the window where a retirement lands between this read and the write.
    //
    //    **It must be here rather than only in SQL**, because two arms below return before the
    //    write function is ever called: the idempotent no-op at step 2 answers `Ok` for a rename to
    //    the name a context already has, so a retired context would report success from a path the
    //    backstop never sees. `CONTEXT_REFUSAL` and nothing more specific — the refusal must not
    //    tell a caller that a retired context sits behind this id when a plain miss would not.
    if !cur.is_active {
        return Err(ApiError::NotFound(CONTEXT_REFUSAL.to_string()));
    }

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
    // query, which has no self-exclusion and correctly needs none — it changes the
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
        // enumerate that owner's contexts. Modelled on `reassign`'s message.
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

/// Retire a context — a soft delete. `kb_contexts.is_active` flips to `false`, which both
/// floored predicates (`contexts_readable_by_teams`, `context_authorable_by_profile`) treat as
/// zero read-reach and zero write authority, while every row it homes is preserved untouched.
/// The slug is mangled in the same statement so the freed address is immediately reusable —
/// `UNIQUE (owner_table, owner_id, slug)` is never relaxed, so the retired row keeps occupying
/// its OLD slug forever unless it moves out of the way.
///
/// **Auth before writes**, the same gate as [`rename`]: `ContextAdminAuthority` — own the context,
/// or manage its owning team, or be an instance administrator outside someone else's personal
/// space (where the admin is refused with a `403` that names it).
///
/// **No dependents guard.** This supersedes the hard delete of PR #777, whose guard existed only
/// because a hard delete could strand a resource's home; retirement strands nothing; the whole
/// point is that a context homing live resources can still be retired.
pub async fn retire(
    pool: &PgPool,
    authed: &Caller,
    context_id: uuid::Uuid,
) -> ApiResult<RetireContextOutcome> {
    let caller = authed.profile_id();
    crate::authz::authorize::<ContextAdminAuthority>(pool, Principal::from(authed), context_id)
        .await?;

    // The current identity pair plus the already-sigil'd owner ref, in one read — copied verbatim
    // from `rename`. `fetch_optional`, not `fetch_one`: the gate's `SystemAdmin` arm
    // admits without consulting the subject's existence, so a system admin naming a context id
    // that does not exist reaches here. That is the read refusal, not a panic. The `CASE` is the
    // incumbent both-kinds spelling (`list_visible`, `get_visible`, `create`) — `team_owner_ref` is
    // team-only and would `fetch_one`-panic on a profile-owned context.
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

    // The mangled address, computed through the incumbent rather than a second uniqueness rule.
    // `next_unique_context_slug` is deliberately `is_active`-BLIND: retired rows keep their slugs
    // in the same UNIQUE space, so a floor added there would hand out an address that collides
    // with a retired row and fail at the INSERT. Do not "fix" it.
    let retired_slug = next_unique_context_slug(
        pool,
        &cur.owner_table,
        cur.owner_id,
        &format!("{}-retired", cur.slug),
    )
    .await?;

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::retire_context_with(
        pool,
        temper_substrate::ids::ContextId::from(context_id),
        cur.slug.as_str(),
        retired_slug.as_str(),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(map_context_write_err)?;

    // Composed from the already-decorated `cur.owner_ref` — never through `decorated_context_ref`,
    // whose `owner_addressable` parameter is the *bare* handle/team slug and would yield
    // `@@handle/slug` from this value. Same note `rename` carries at its own return.
    Ok(RetireContextOutcome {
        context_id: ContextId::from(context_id),
        context_ref: format!("{}/{retired_slug}", cur.owner_ref),
        slug: retired_slug,
        name: cur.name,
    })
}

/// Restore a retired context — the reverse of [`retire`]. Re-derives the address from the
/// untouched `name` rather than trying to recover whatever retire mangled the slug to; that
/// mangled slug was chosen only to get out of the way, and is not owed back.
///
/// **Auth before writes**, the same gate as [`retire`]: `ContextAdminAuthority` — own the
/// context, or manage its owning team, or be an instance administrator outside someone else's
/// personal space (where the admin is refused with a `403` that names it).
///
/// **An erased context is refused with 410**, whoever asks: the person-erasure act erased it,
/// and erased stays erased. Checked here so the caller gets the refusal before any slug is
/// derived; `context_restore` refuses it again in its own transaction (SQLSTATE `TE001`).
///
/// **Fetching `cur` here is the subtle part.** The context is retired, so anything that reads
/// through the read predicate will not find it. This copies `retire`'s own `cur` fetch
/// verbatim — it is already `is_active`-BLIND because it reads `kb_contexts` by
/// primary key, not through `context_visible_to`.
pub async fn restore(
    pool: &PgPool,
    authed: &Caller,
    context_id: uuid::Uuid,
) -> ApiResult<RestoreContextOutcome> {
    let caller = authed.profile_id();
    let on_admin_arm = context_admin_arm(pool, authed, context_id).await?;

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

    let erased = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'context_erased'
                  AND e.producing_anchor_table = 'kb_contexts'
                  AND e.producing_anchor_id = $1) AS "erased!""#,
        context_id,
    )
    .fetch_one(pool)
    .await?;
    if erased {
        return Err(ApiError::Gone(CONTEXT_ERASED.to_string()));
    }

    // Re-derived from the untouched name, not the mangled retired slug — `next_unique_context_slug`
    // is `is_active`-BLIND for the same reason `retire`'s call is: the freed-then-reclaimed slug
    // space is shared with retired rows.
    let restored_slug =
        next_unique_context_slug(pool, &cur.owner_table, cur.owner_id, &cur.name).await?;

    // The address the caller retired UNDER, read off the ledger entry that recorded it: [`retire`]
    // passes `cur.slug` to `retire_context_with` as `from_slug`, which is exactly the pre-mangle
    // address and rides in the `context_retired` payload.
    //
    // Anchor-scoped, the shape `event_service::latest_event_id_for_context` uses (`_event_append`
    // anchors context events to `('kb_contexts', context_id)`), most recent first because a
    // context can be retired and restored more than once and only the last retirement is the one
    // being undone.
    let retired_from_slug = sqlx::query_scalar!(
        r#"SELECT e.payload->>'from_slug' AS from_slug
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
            WHERE t.name = 'context_retired'
              AND e.producing_anchor_table = 'kb_contexts'
              AND e.producing_anchor_id = $1
            ORDER BY e.occurred_at DESC, e.id DESC
            LIMIT 1"#,
        context_id,
    )
    .fetch_optional(pool)
    .await?
    .flatten();

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut tx = context_admin_write_tx(pool, authed, context_id, on_admin_arm).await?;
    temper_substrate::writes::restore_context_on(
        &mut tx,
        temper_substrate::ids::ContextId::from(context_id),
        cur.slug.as_str(),
        restored_slug.as_str(),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(map_context_write_err)?;
    tx.commit().await?;

    // The baseline is the retired-under address, NOT `sluggify(&cur.name)`. `create` hands out
    // suffixed slugs for duplicate names, so a context named "Notes" routinely lives at `notes-2`,
    // and the name's canonical slug then answers a question nobody asked — it calls a restore that
    // landed exactly where it started "changed", and, worse, calls a restore that handed back a
    // DIFFERENT address "unchanged". The second is the silent re-addressing this field exists to
    // refuse. Witnessed by `restore_reports_a_change_when_it_lands_on_a_freed_sibling_address` and
    // `restore_reports_no_change_when_it_lands_back_on_its_suffixed_address`.
    //
    // No ledger entry: the context was retired by the un-evented path that shipped earlier on this
    // branch, before migration `20260826000120` added `context_retire`. The address it was retired
    // under is recorded nowhere, so the answer that cannot mislead is "assume it moved" — `true`
    // sends the caller to the `slug` this same response carries, while `false` would assert an
    // address was preserved that nothing on disk can vouch for. The arm is bounded rather than
    // open-ended: every retirement since that migration appends the event, so it only ever covers
    // rows retired before it.
    let slug_changed = retired_from_slug.is_none_or(|from| restored_slug != from);

    // Composed from the already-decorated `cur.owner_ref` — never through `decorated_context_ref`,
    // whose `owner_addressable` parameter is the *bare* handle/team slug and would yield
    // `@@handle/slug` from this value. Same note `retire` carries at its own return.
    Ok(RestoreContextOutcome {
        context_id: ContextId::from(context_id),
        context_ref: format!("{}/{restored_slug}", cur.owner_ref),
        slug: restored_slug,
        name: cur.name,
        slug_changed,
    })
}

/// The `23505` refusal both context write paths render when they lose the collision race.
///
/// A constant rather than a literal, for the reason [`CONTEXT_REFUSAL`] gives: it is one defence
/// shared by two call sites, and two copies would be two defences that drift. It deliberately names
/// neither the owner nor the slug — unlike the pre-check messages, which have a row in hand, this
/// one is reconstructed from a SQLSTATE and has nothing but the constraint that fired.
const CONTEXT_SLUG_TAKEN: &str = "that owner already holds a context with this slug";

/// The two-sided gate's decision on the pool, and whether it rested on the system-admin arm.
pub(crate) async fn two_sided_admin_arm(
    pool: &PgPool,
    authed: &HumanPrincipal,
    scope: TwoSidedScope,
) -> ApiResult<bool> {
    Ok(
        crate::authz::authorize::<TwoSidedAuthority>(pool, Principal::Human(authed), scope)
            .await?
            .authority()
            == TwoSidedAuthority::SystemAdmin,
    )
}

/// The transaction a two-sided act writes on, opened with the admin re-check first. A decision on
/// the system-admin arm was a pool read of governance; it is asked again under the governance lock
/// (`auth::is_system_admin_locked`, held to the commit) before the write, so an admin demoted since
/// (or while this runs) changes nothing. The refusal is the gate's own answer to the caller as
/// they now stand, decided again on the pool, so a demoted admin is told what a non-admin is told;
/// one the gate now admits on another arm (a delegate) is refused this once with `403`, and a retry
/// decides on that arm. Lock order: governance, then the share, binding or context row the act
/// writes; a demotion locks none of those.
pub(crate) async fn two_sided_write_tx(
    pool: &PgPool,
    authed: &HumanPrincipal,
    scope: TwoSidedScope,
    on_admin_arm: bool,
) -> ApiResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    if on_admin_arm && !crate::auth::is_system_admin_locked(&mut tx, authed.profile_id()).await? {
        drop(tx);
        crate::authz::authorize::<TwoSidedAuthority>(pool, Principal::Human(authed), scope).await?;
        return Err(ApiError::Forbidden);
    }
    Ok(tx)
}

/// [`ContextAdminAuthority`]'s decision on the pool, and whether it rested on the system-admin arm.
async fn context_admin_arm(
    pool: &PgPool,
    authed: &Caller,
    context_id: uuid::Uuid,
) -> ApiResult<bool> {
    Ok(
        crate::authz::authorize::<ContextAdminAuthority>(pool, Principal::from(authed), context_id)
            .await?
            .authority()
            == ContextAdminAuthority::SystemAdmin,
    )
}

/// [`two_sided_write_tx`] for restore, the one context-administration act with a durable effect: a
/// restored context's write grants and authoring arms (`context_authorable_by_profile` reads
/// `is_active`, and every explicit grant `context_is_live`) come back with it. The SQL writer asks
/// `is_system_admin` again after its context lock; on this transaction that read is taken under the
/// governance row this has locked, so its answer holds to the commit too. The refusal is
/// [`ContextAdminAuthority`]'s answer to the caller as they now stand: `403` to a reader, the
/// not-found sentence to anyone who cannot read the context. Rename and retire grant no reach and
/// are not re-checked.
async fn context_admin_write_tx(
    pool: &PgPool,
    authed: &Caller,
    context_id: uuid::Uuid,
    on_admin_arm: bool,
) -> ApiResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    if on_admin_arm && !crate::auth::is_system_admin_locked(&mut tx, authed.profile_id()).await? {
        drop(tx);
        crate::authz::authorize::<ContextAdminAuthority>(pool, Principal::from(authed), context_id)
            .await?;
        return Err(ApiError::Forbidden);
    }
    Ok(tx)
}

/// Map a substrate write error from `reassign_context_on` / `rename_context_with` /
/// `retire_context_with` / `restore_context_on` to an [`ApiError`]. **One mapper, four call
/// sites** — the mapping is identical for all of them and would drift
/// if written twice.
///
/// - `42501` (insufficient_privilege) — `context_reassign`, `context_rename`, `context_retire`, and
///   `context_restore` each carry their RBAC gate as an in-transaction invariant, not merely a
///   caller pre-check. The Rust gate renders a clean refusal on the common path, so this arm only
///   fires on a TOCTOU change between check and write; that lost race should still read as `403`,
///   not `500`.
/// - `23505` (unique_violation) — `UNIQUE (owner_table, owner_id, slug)` is the backstop behind
///   every call site's slug-collision pre-check. **The race path must render the same refusal as the
///   pre-check**, or the caller's experience depends on how quickly they lost the race rather than
///   on the state of the system: the caller who won gets `409` and the caller who lost gets `500`
///   for the same conflict. `reassign` shipped with exactly this hole — a `42501`-only body — and
///   rename's tests are what surfaced it; it is fixed here rather than left beside a correct copy.
/// - `P0002` (no_data_found) — `context_retire`/`context_restore` each refuse a no-op mutation
///   (retiring an already-retired context, restoring an already-active one) rather than append a
///   spurious event, raising `P0002`. Rendered as the same `CONTEXT_REFUSAL` `404` the pre-event
///   `rows_affected() == 0` check used to render for this exact case.
///
/// Everything else is a genuine internal error.
fn map_context_write_err(e: anyhow::Error) -> ApiError {
    if let Some(sqlx::Error::Database(db)) = e.downcast_ref::<sqlx::Error>() {
        match db.code().as_deref() {
            Some("42501") => return ApiError::Forbidden,
            Some("23505") => return ApiError::Conflict(CONTEXT_SLUG_TAKEN.to_string()),
            Some("P0002") => return ApiError::NotFound(CONTEXT_REFUSAL.to_string()),
            Some("TE001") => return ApiError::Gone(CONTEXT_ERASED.to_string()),
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

    /// The no-op refusal `context_retire`/`context_restore` raise (retiring an already-retired
    /// context, restoring an already-active one) renders the same `CONTEXT_REFUSAL` 404 the
    /// pre-event `rows_affected() == 0` check used to render for this exact case.
    #[test]
    fn no_data_found_renders_the_same_not_found_the_old_rows_affected_check_rendered() {
        match map_context_write_err(substrate_err("P0002")) {
            ApiError::NotFound(msg) => assert_eq!(msg, CONTEXT_REFUSAL),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// `context_restore`'s refusal of an erased context, lost to the race the service's own
    /// pre-check cannot close (the act committed between the two), renders the same 410 the
    /// pre-check does.
    #[test]
    fn an_erased_context_refusal_renders_gone() {
        match map_context_write_err(substrate_err("TE001")) {
            ApiError::Gone(msg) => assert_eq!(msg, CONTEXT_ERASED),
            other => panic!("expected Gone, got {other:?}"),
        }
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
mod tests;
