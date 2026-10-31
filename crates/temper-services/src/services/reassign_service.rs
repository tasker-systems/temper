//! Resource ownership reassignment over `kb_resource_homes`, event-sourced via
//! `writes::reassign_resource_with`. Service-direct (no Backend-trait command),
//! same precedent as `invitation_service` / `team_service`. Authorization
//! precedes every write.
//!
//! Two authorized paths: the current owner may reassign their own resource to
//! anyone (mis-attribution self-fix); a team admin may reassign a resource
//! homed in a context their team owns, owned by a current or former member of it, to a member
//! of that team (offboarding +
//! admin-assisted mis-attribution). Only **context-homed** resources are
//! reassignable — a cogmap interior is team-resource-derived, not personally
//! owned, and is refused here (and by the `resource_reassign` SQL backstop).

use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::HumanPrincipal;
use crate::backend::write_floor;
use crate::error::{ApiError, ApiResult};
use crate::services::team_service::{can_manage, role_on_team};
use temper_core::error::TemperError;
use temper_core::types::ids::ProfileId;

/// A resource's home owner + the anchor table it's homed under.
struct HomeRow {
    owner: Uuid,
    anchor_table: String,
}

async fn home_of(pool: &PgPool, resource: Uuid) -> ApiResult<Option<HomeRow>> {
    Ok(sqlx::query_as!(
        HomeRow,
        "SELECT owner_profile_id AS owner, anchor_table \
           FROM kb_resource_homes WHERE resource_id = $1",
        resource,
    )
    .fetch_optional(pool)
    .await?)
}

/// Is there a team T where caller manages T, `resource` is homed in a context **owned by T**, `to` is a
/// member of T, and the resource's current owner is a current or former member of T? T is never the
/// root or the gating team. (admin-path reach: from-scope + into-scope.)
///
/// Both the team and the context must be live. A retired context closes this arm the
/// same way it closes the team-share read arm in `contexts_readable_by_teams` — without
/// that floor, an admin whose only reach was the team share could reassign a retired
/// context's resource to themselves and then read it through the owner arm of
/// `resources_visible_to`, which has no context floor. Witness:
/// `admin_cannot_reassign_via_retired_context`.
///
/// **Reassign acts only inside T's own containers** (team-authority fixes D5). A read-only share of
/// someone's context into T used to let T's managers take ownership of the resources in it. A
/// share confers read and nothing more, so only contexts T owns are in scope: the one place T's
/// managers already govern the container. Membership is no bar on its own — a manager can add
/// anyone to T — so it is kept as an extra check, not as the authority.
async fn admin_reach(
    pool: &PgPool,
    caller: ProfileId,
    resource: Uuid,
    to: Uuid,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM kb_resource_homes h
            JOIN kb_contexts c
              ON c.id = h.anchor_id AND c.is_active
            JOIN kb_teams t
              ON t.is_active
             AND c.owner_table = 'kb_teams' AND c.owner_id = t.id
             AND t.slug <> 'temper-system'
             AND t.slug IS DISTINCT FROM
                 (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)
            JOIN kb_team_members cm
              ON cm.team_id = t.id AND cm.profile_id = $2
                 AND cm.role IN ('owner', 'maintainer')
            JOIN kb_team_members tm
              ON tm.team_id = t.id AND tm.profile_id = $3
            WHERE h.resource_id = $1
              AND h.anchor_table = 'kb_contexts'
              AND ( EXISTS (SELECT 1 FROM kb_team_members om
                             WHERE om.team_id = t.id AND om.profile_id = h.owner_profile_id)
                 OR EXISTS (SELECT 1 FROM kb_events e
                              JOIN kb_event_types et ON et.id = e.event_type_id
                             WHERE et.name = 'team_membership_changed'
                               AND e.payload @> jsonb_build_object(
                                     'team_id', t.id, 'profile_id', h.owner_profile_id)) )
        ) AS "exists!: bool"
        "#,
        resource,
        *caller,
        to,
    )
    .fetch_one(pool)
    .await?)
}

/// Is `profile_id` a current member of `team_id`, or a former one — a `team_membership_changed`
/// admin event names them on it? The "former" half is what offboarding needs: the handoff runs
/// after the member is gone. A removal that predates the ledger (`20261101200000`) left no record,
/// so that leaver is reassigned with the admin path instead.
async fn is_or_was_member(pool: &PgPool, team_id: Uuid, profile_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT ( EXISTS (SELECT 1 FROM kb_team_members
                             WHERE team_id = $1 AND profile_id = $2)
                 OR EXISTS (SELECT 1 FROM kb_events e
                              JOIN kb_event_types et ON et.id = e.event_type_id
                             WHERE et.name = 'team_membership_changed'
                               AND e.payload @> jsonb_build_object(
                                     'team_id', $1::uuid, 'profile_id', $2::uuid)) ) AS "e!: bool""#,
        team_id,
        profile_id,
    )
    .fetch_one(pool)
    .await?)
}

/// Reassign a resource's owner to `to_profile_id`. Auth: current owner (any target)
/// OR team-admin over a team the resource is scoped to, to a member of that team.
/// Reassigning to the current owner is an idempotent no-op. Cogmap-homed resources
/// are rejected (map interiors are not personally owned).
///
/// Authority is decided FIRST, and an id with no home is refused exactly as an existing resource
/// the caller has no authority over: `403`, the same body. Nothing the door answers before the
/// authority check may depend on the id — otherwise the status says whether the id is a resource,
/// and what kind of home it has, to any caller.
pub async fn reassign_resource(
    pool: &PgPool,
    caller: &HumanPrincipal,
    resource_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<()> {
    let caller = caller.profile_id();
    // Auth before writes: current owner, or an admin with reach over the resource+target. An
    // unknown id has no owner and no reach, so it is refused like any other resource.
    let Some(home) = home_of(pool, resource_id).await? else {
        return Err(ApiError::Forbidden);
    };
    let authorized =
        home.owner == *caller || admin_reach(pool, caller, resource_id, to_profile_id).await?;
    if !authorized {
        return Err(ApiError::Forbidden);
    }

    // Only context-homed resources are reassignable. The owner path would otherwise
    // let a cogmap-node owner flip it — guard here for BOTH paths (the admin path's
    // reach query already excludes non-context homes structurally, so only an owner
    // reaches this refusal).
    if home.anchor_table != "kb_contexts" {
        return Err(ApiError::BadRequest(
            "cannot reassign a cogmap-homed resource; map interiors are not personally owned"
                .to_string(),
        ));
    }

    // The liveness floor (resource erasure spec D13; plan 2c, controller ruling 5), at the head of
    // the reassign's own transaction: the resource must be live, under the `FOR KEY SHARE` row
    // lock the erasure act's `FOR UPDATE` conflicts with, held until the reassign commits. Liveness
    // only — the authority stays the gate above. A refusal is classified as every write floor's
    // is: 410 `RESOURCE_ERASED` to a holder of an erased husk, 403 otherwise (a tombstone always
    // 403). It runs ahead of the no-op return so a husk is never answered `200`.
    let mut tx = pool.begin().await?;
    if let Err(refusal) = write_floor::liveness_floor_in_tx(
        &mut tx,
        caller,
        temper_substrate::ids::ResourceId::from(resource_id),
    )
    .await
    {
        return Err(write_floor::rollback_with(tx, ApiError::from(refusal)).await);
    }
    if home.owner == to_profile_id {
        return Ok(()); // idempotent no-op
    }

    // The owner path admits any target; verify it's a real profile so a bad UUID is a
    // clean 400 rather than an FK-violation 500 in the projector. (The admin path's
    // membership join already guarantees existence, but this covers both uniformly.) This and the
    // emitter below run on the reassign's own transaction, never the pool: the transaction already
    // holds a pool connection, and a second acquire while it is open is how concurrent writers
    // stall the pool.
    let to_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE id = $1) AS "e!: bool""#,
        to_profile_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    if !to_exists {
        return Err(ApiError::BadRequest(
            "target profile does not exist".to_string(),
        ));
    }

    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller, "web")
        .await
        .map_err(ApiError::from_write)?;
    temper_substrate::writes::reassign_resource_in_tx(
        &mut tx,
        temper_substrate::ids::ResourceId::from(resource_id),
        home.owner.into(),
        to_profile_id.into(),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(ApiError::from_write)?;
    tx.commit().await?;
    Ok(())
}

/// A resource owned by a given profile and homed in a context owned by a given team. The single definition of "what a departing member still owns in this
/// team" — consumed by both the bulk handoff (the move set) and `remove_member`'s
/// residual surfacing (the count + per-context breakdown), so the two can never drift.
pub struct ScopedOwnedRow {
    pub resource_id: Uuid,
    pub context_id: Uuid,
    /// Decorated context ref `{owner_ref}/{slug}` (owner_ref: `@handle` | `+slug`).
    pub context_ref: String,
}

/// *Live* resources owned by `profile_id` and homed in a *live* context OWNED by `team_id`. A context
/// merely shared to the team is out of scope: a share confers read, never authority over the
/// container's contents (team-authority fixes D5).
/// A retired context drops out of this scope, and so does a soft-deleted or erased resource
/// (`kb_resources.is_active`), so the bulk handoff and `remove_member`'s residual surfacing
/// both stop counting either and agree on what is left to hand off. Witnesses:
/// `bulk_excludes_a_retired_contexts_resources`, and in `resource_husk_write_test.rs`
/// `remove_member_and_team_reassign_leave_out_a_husk_and_a_tombstone`.
///
/// Ordered by `(context slug, resource_id)` for stable output only — consumers
/// that group by context MUST key on `context_id` (a slug is unique only per-owner,
/// so two distinct contexts can share a slug and are NOT guaranteed to be adjacent).
pub async fn team_scoped_owned(
    pool: &PgPool,
    team_id: Uuid,
    profile_id: Uuid,
) -> ApiResult<Vec<ScopedOwnedRow>> {
    let rows = sqlx::query!(
        r#"
        SELECT h.resource_id AS "resource_id!",
               c.id          AS "context_id!",
               c.slug        AS "slug!",
               CASE c.owner_table
                 WHEN 'kb_teams'    THEN '+' || t.slug
                 WHEN 'kb_profiles' THEN '@' || p.handle
               END AS "owner_ref!"
        FROM kb_contexts c
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
        JOIN kb_resources r ON r.id = h.resource_id AND r.is_active
        LEFT JOIN kb_teams    t ON c.owner_table = 'kb_teams'    AND t.id = c.owner_id
        LEFT JOIN kb_profiles p ON c.owner_table = 'kb_profiles' AND p.id = c.owner_id
        WHERE c.is_active
          AND h.owner_profile_id = $2
          -- Only the team's OWN contexts (team-authority fixes D5): a share confers read, never
          -- authority over what is in the shared container.
          AND c.owner_table = 'kb_teams' AND c.owner_id = $1
        ORDER BY c.slug, h.resource_id
        "#,
        team_id,
        profile_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScopedOwnedRow {
            resource_id: r.resource_id,
            context_id: r.context_id,
            context_ref: format!("{}/{}", r.owner_ref, r.slug),
        })
        .collect())
}

/// Bulk-reassign, from `from_profile_id` to `to_profile_id`, every LIVE resource owned by
/// `from` and homed in a context owned by `team_id`. Auth: caller manages the team (never the root
/// or the gating team), `to` is a member of it, and `from` is or was a member of it. One transaction; returns the reassigned resource ids. A tombstoned or
/// erased resource is outside the scope; one tombstoned or erased after the scope read fails the
/// liveness floor, is skipped, and is not among them.
pub async fn reassign_team_resources(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    from_profile_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<Vec<Uuid>> {
    let caller = caller.profile_id();
    // A soft-deleted team is inert — it confers no reassignment authority. `role_on_team` is
    // live-only too; this check stays because it answers `Forbidden` before any lookup by role.
    let team_active = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1 AND is_active) AS "e!: bool""#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    if !team_active {
        return Err(ApiError::Forbidden);
    }

    // Auth before writes: caller manages the team, and `to` is a member of it. Never on the root or
    // the gating team, where a leftover managing row (a demoted admin's) confers nothing.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    let root_or_gating = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM kb_teams t
              WHERE t.id = $1
                AND (t.slug = 'temper-system'
                     OR t.slug IS NOT DISTINCT FROM
                        (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)))
           AS "e!: bool""#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    if root_or_gating {
        return Err(ApiError::Forbidden);
    }
    let to_is_member = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_team_members WHERE team_id=$1 AND profile_id=$2) AS "e!: bool""#,
        team_id,
        to_profile_id,
    )
    .fetch_one(pool)
    .await?;
    if !to_is_member {
        return Err(ApiError::Forbidden);
    }
    // ...and `from` is, or was, on the team: reassign hands off a member's work, never a sharer's
    // (team-authority fixes D5).
    if !is_or_was_member(pool, team_id, from_profile_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Scope read: the shared definition (owned by `from` ∩ homed in a team-shared context).
    let targets: Vec<Uuid> = team_scoped_owned(pool, team_id, from_profile_id)
        .await?
        .into_iter()
        .map(|r| r.resource_id)
        .collect();
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(ApiError::from_write)?;

    let mut tx = pool.begin().await?;
    let mut reassigned = Vec::with_capacity(targets.len());
    for &rid in &targets {
        let resource = temper_substrate::ids::ResourceId::from(rid);
        // The liveness floor per resource (controller ruling 5), in the run's transaction, ahead
        // of its write. `team_scoped_owned` enumerates live resources only, so the floor refuses
        // one erased or deleted after the scope read. It is SKIPPED — left with its owner and
        // absent from the returned ids — never a whole-run failure. Only the floor's two refusals
        // skip; a fault aborts the run as before.
        match write_floor::liveness_floor_bulk_in_tx(&mut tx, caller, resource).await {
            Ok(()) => {}
            Err(TemperError::Forbidden | TemperError::ResourceErased(_)) => continue,
            Err(e) => return Err(e.into()),
        }
        temper_substrate::writes::reassign_resource_in_tx(
            &mut tx,
            resource,
            from_profile_id.into(),
            to_profile_id.into(),
            emitter,
            temper_substrate::events::EventContext::default(),
        )
        .await
        .map_err(ApiError::from_write)?;
        reassigned.push(rid);
    }
    tx.commit().await?;
    Ok(reassigned)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    async fn mk_profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        // Every profile needs a `<handle>@web` emitter entity for resolve_emitter.
        sqlx::query("INSERT INTO kb_entities (name, profile_id) VALUES ($1 || '@web', $2)")
            .bind(handle)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        ProfileId::from(id)
    }

    async fn mk_context(pool: &PgPool, slug: &str, owner: ProfileId) -> Uuid {
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

    /// A context OWNED by `team` — the only kind a team's reassign reaches (team-authority fixes D5).
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
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('r','r') RETURNING id",
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

    /// A resource homed in a cogmap (map interior). `anchor_id` needs no real cogmap row —
    /// `kb_resource_homes.anchor_id` has no FK, and the reassign guard only inspects `anchor_table`.
    async fn mk_cogmap_homed_resource(pool: &PgPool, owner: ProfileId) -> Uuid {
        let rid: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('node','node') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO kb_resource_homes \
               (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1, 'kb_cogmaps', uuid_generate_v7(), $2, $2)",
        )
        .bind(rid)
        .bind(*owner)
        .execute(pool)
        .await
        .unwrap();
        rid
    }

    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
        )
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn add_member(pool: &PgPool, team: Uuid, p: ProfileId, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1,$2,$3::team_role,'native'::team_member_source)",
        )
        .bind(team)
        .bind(*p)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn share_ctx(pool: &PgPool, ctx: Uuid, team: Uuid) {
        sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1,$2)")
            .bind(ctx)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn soft_delete_team(pool: &PgPool, team: Uuid) {
        sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn retire_ctx(pool: &PgPool, ctx: Uuid) {
        sqlx::query("UPDATE kb_contexts SET is_active = false WHERE id = $1")
            .bind(ctx)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn owner_of(pool: &PgPool, resource: Uuid) -> Uuid {
        sqlx::query_scalar!(
            "SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id=$1",
            resource
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn visible_to(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
        sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2) AS \"e!: bool\"",
            *profile,
            resource,
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_scoped_owned_matches_bulk_scope(pool: PgPool) {
        // Same fixture shape as `bulk_reassigns_only_owned_and_scoped`.
        let leaver = mk_profile(&pool, "leaver").await;
        let other = mk_profile(&pool, "other").await;
        let team = mk_team(&pool, "acme").await;

        let shared = mk_team_context(&pool, "shared", team).await;
        let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

        let in_scope = mk_homed_resource(&pool, shared, leaver).await; // owned + scoped → listed
        let _out_scope = mk_homed_resource(&pool, private, leaver).await; // owned, not scoped → excluded
        let _not_leaver = mk_homed_resource(&pool, shared, other).await; // scoped, other owner → excluded

        let rows = team_scoped_owned(&pool, team, *leaver)
            .await
            .expect("scope read");
        let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.resource_id).collect();
        assert_eq!(ids, vec![in_scope]);
        assert!(
            rows[0].context_ref.ends_with("/shared"),
            "decorated ref: {}",
            rows[0].context_ref
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_can_reassign_and_visibility_follows(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "alice-ctx", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *alice).await,
            r,
            *bob,
        )
        .await
        .expect("owner reassigns");

        assert_eq!(owner_of(&pool, r).await, *bob);
        assert!(visible_to(&pool, bob, r).await, "new owner sees it");
        assert!(
            visible_to(&pool, alice, r).await,
            "originator retains access"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reassign_emits_event(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *alice).await,
            r,
            *bob,
        )
        .await
        .unwrap();
        let n = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
             WHERE t.name='resource_reassigned' AND (e.payload->>'resource_id')::uuid=$1",
            r,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
        assert_eq!(n, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reassign_to_current_owner_is_noop(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *alice).await,
            r,
            *alice,
        )
        .await
        .expect("idempotent no-op");
        assert_eq!(owner_of(&pool, r).await, *alice);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn stranger_cannot_reassign(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let mallory = mk_profile(&pool, "mallory").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *mallory).await,
            r,
            *bob,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    /// FAILS IF the door answers a caller without authority differently by what the id is. An
    /// unknown id, another's context-homed resource and another's cogmap-homed resource each refuse
    /// the stranger the same `Forbidden` — no status names whether the id is a resource or what
    /// kind of home it has. The bite: read the home with a `NotFound` for a missing row, or run the
    /// cogmap-home `BadRequest` ahead of the authority check, in `reassign_resource`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_stranger_is_refused_alike_whatever_the_id_is(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let mallory = mk_profile(&pool, "mallory").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let in_context = mk_homed_resource(&pool, ctx, alice).await;
        let in_cogmap = mk_cogmap_homed_resource(&pool, alice).await;
        for (what, id) in [
            ("an unknown id", Uuid::now_v7()),
            ("another's context-homed resource", in_context),
            ("another's cogmap-homed resource", in_cogmap),
        ] {
            let err = reassign_resource(
                &pool,
                &crate::test_support::human_principal_for(&pool, *mallory).await,
                id,
                *mallory,
            )
            .await
            .unwrap_err();
            assert!(matches!(err, ApiError::Forbidden), "{what}: {err:?}");
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_reassign_cogmap_homed_resource_even_as_owner(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let r = mk_cogmap_homed_resource(&pool, alice).await;
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *alice).await,
            r,
            *bob,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_admin_can_reassign_scoped_resource_to_member(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await; // current owner + departing
        let admin = mk_profile(&pool, "admin").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        add_member(&pool, team, alice, "member").await;
        let ctx = mk_team_context(&pool, "shared", team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *steward,
        )
        .await
        .expect("admin reassigns to member");
        assert_eq!(owner_of(&pool, r).await, *steward);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn admin_cannot_reassign_to_non_member(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let admin = mk_profile(&pool, "admin").await;
        let outsider = mk_profile(&pool, "outsider").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, alice, "member").await;
        let ctx = mk_team_context(&pool, "shared", team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *outsider,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn admin_cannot_reassign_unscoped_resource(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let admin = mk_profile(&pool, "admin").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        let ctx = mk_context(&pool, "private", alice).await; // NOT shared
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *steward,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_reassign_to_nonexistent_profile_is_bad_request(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let ctx = mk_context(&pool, "c", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let ghost = Uuid::now_v7(); // never inserted
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *alice).await,
            r,
            ghost,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn admin_cannot_reassign_via_soft_deleted_team(pool: PgPool) {
        // Same shape as the passing admin case, but the team is soft-deleted → inert.
        let alice = mk_profile(&pool, "alice").await;
        let admin = mk_profile(&pool, "admin").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, alice, "member").await;
        add_member(&pool, team, steward, "member").await;
        let ctx = mk_team_context(&pool, "shared", team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        soft_delete_team(&pool, team).await;

        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *steward,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
        assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_forbidden_on_soft_deleted_team(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, leaver, "member").await;
        add_member(&pool, team, steward, "member").await;
        let shared = mk_team_context(&pool, "shared", team).await;
        let r = mk_homed_resource(&pool, shared, leaver).await;
        soft_delete_team(&pool, team).await;

        let err = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *leaver,
            *steward,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
        assert_eq!(owner_of(&pool, r).await, *leaver, "owner unchanged");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn admin_cannot_reassign_via_retired_context(pool: PgPool) {
        // Same shape as the passing admin case, but the CONTEXT is retired. Retirement
        // revokes the team-share arm; the reassign must not hand it back as ownership.
        let alice = mk_profile(&pool, "alice").await;
        let admin = mk_profile(&pool, "admin").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, alice, "member").await;
        add_member(&pool, team, steward, "member").await;
        let ctx = mk_team_context(&pool, "shared", team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        retire_ctx(&pool, ctx).await;

        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *steward,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
        assert_eq!(owner_of(&pool, r).await, *alice, "owner unchanged");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn owner_can_still_reassign_out_of_a_retired_context(pool: PgPool) {
        // Anti-trap: retirement floors the ADMIN arm only. An owner must keep being able
        // to hand off their own work after the context is retired.
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "alice-ctx", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        retire_ctx(&pool, ctx).await;

        reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *alice).await,
            r,
            *bob,
        )
        .await
        .expect("owner reassigns out of a retired context");
        assert_eq!(owner_of(&pool, r).await, *bob);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_excludes_a_retired_contexts_resources(pool: PgPool) {
        // The team is live, so the call is authorized and succeeds — with a reduced scope.
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        add_member(&pool, team, leaver, "member").await;

        let live = mk_team_context(&pool, "live", team).await;
        let retired = mk_team_context(&pool, "retired", team).await;

        let in_scope = mk_homed_resource(&pool, live, leaver).await; // live share → moves
        let out_scope = mk_homed_resource(&pool, retired, leaver).await; // retired → stays
        retire_ctx(&pool, retired).await;

        let moved = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *leaver,
            *steward,
        )
        .await
        .expect("live team, reduced scope");

        assert_eq!(moved, vec![in_scope]);
        assert_eq!(owner_of(&pool, in_scope).await, *steward);
        assert_eq!(owner_of(&pool, out_scope).await, *leaver, "retired stays");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_reassigns_only_owned_and_scoped(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let other = mk_profile(&pool, "other").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        add_member(&pool, team, leaver, "member").await;

        let shared = mk_team_context(&pool, "shared", team).await;
        let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

        let in_scope = mk_homed_resource(&pool, shared, leaver).await; // owned+scoped → moves
        let out_scope = mk_homed_resource(&pool, private, leaver).await; // owned, not scoped → stays
        let not_leaver = mk_homed_resource(&pool, shared, other).await; // scoped, other owner → stays

        let moved = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *leaver,
            *steward,
        )
        .await
        .expect("bulk reassign");

        assert_eq!(moved, vec![in_scope]);
        assert_eq!(owner_of(&pool, in_scope).await, *steward);
        assert_eq!(owner_of(&pool, out_scope).await, *leaver);
        assert_eq!(owner_of(&pool, not_leaver).await, *other);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_non_manager_forbidden(pool: PgPool) {
        let stranger = mk_profile(&pool, "stranger").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, steward, "member").await;
        let err = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *stranger).await,
            team,
            *leaver,
            *steward,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_into_non_member_forbidden(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let outsider = mk_profile(&pool, "outsider").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        let err = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *leaver,
            *outsider,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bulk_empty_match_is_ok(pool: PgPool) {
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        add_member(&pool, team, leaver, "member").await;
        let moved = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *leaver,
            *steward,
        )
        .await
        .expect("empty is ok");
        assert!(moved.is_empty());
    }

    // ---- Reassign acts only inside the team's own contexts (team-authority fixes D5). ----

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_forced_member_cannot_be_reassigned_out_of_their_own_teams_shared_context(
        pool: PgPool,
    ) {
        // The shape that defeated the first D5: Bob owns team B, B's context is shared read-only
        // into T, and T's manager Mallory force-adds Bob to T (add_member needs no consent). Bob's
        // resource in B's context stays Bob's.
        let mallory = mk_profile(&pool, "mallory").await;
        let bob = mk_profile(&pool, "bob").await;
        let t = mk_team(&pool, "acme").await;
        let b = mk_team(&pool, "bobs").await;
        add_member(&pool, t, mallory, "owner").await;
        add_member(&pool, b, bob, "owner").await;
        let ctx = mk_team_context(&pool, "b-notes", b).await;
        share_ctx(&pool, ctx, t).await;
        let r = mk_homed_resource(&pool, ctx, bob).await;
        add_member(&pool, t, bob, "watcher").await;

        let m = crate::test_support::human_principal_for(&pool, *mallory).await;
        let err = reassign_resource(&pool, &m, r, *mallory).await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden), "{err:?}");
        let moved = reassign_team_resources(&pool, &m, t, *bob, *mallory)
            .await
            .expect("authorized, empty scope");
        assert!(moved.is_empty());
        assert_eq!(owner_of(&pool, r).await, *bob);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_shared_context_is_never_in_scope(pool: PgPool) {
        // Bob shares HIS context read-only into the team. Whether or not he is a member, the
        // team's managers cannot take his resources in it: a share confers read, never authority
        // over the shared container's contents.
        let admin = mk_profile(&pool, "admin").await;
        let bob = mk_profile(&pool, "bob").await;
        let mallory = mk_profile(&pool, "mallory").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, mallory, "maintainer").await;
        let ctx = mk_context(&pool, "bob-notes", bob).await;
        share_ctx(&pool, ctx, team).await;
        let r = mk_homed_resource(&pool, ctx, bob).await;

        // Not a member: refused outright, on both doors.
        let err = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *bob,
            *mallory,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden), "{err:?}");
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *mallory,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden), "{err:?}");

        // A member: still never his own context's resources.
        add_member(&pool, team, bob, "member").await;
        let moved = reassign_team_resources(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            team,
            *bob,
            *mallory,
        )
        .await
        .expect("authorized, empty scope");
        assert!(moved.is_empty());
        let err = reassign_resource(
            &pool,
            &crate::test_support::human_principal_for(&pool, *admin).await,
            r,
            *mallory,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden), "{err:?}");
        assert_eq!(owner_of(&pool, r).await, *bob);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_former_member_recorded_on_the_ledger_is_handed_off(pool: PgPool) {
        // Offboarding proper: the leaver is removed through the recorded membership write, then
        // their work in a TEAM-OWNED context (in scope since D5) is handed to a steward.
        let admin = mk_profile(&pool, "admin").await;
        let leaver = mk_profile(&pool, "leaver").await;
        let steward = mk_profile(&pool, "steward").await;
        let team = mk_team(&pool, "acme").await;
        add_member(&pool, team, admin, "owner").await;
        add_member(&pool, team, steward, "member").await;
        add_member(&pool, team, leaver, "member").await;
        let team_ctx: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_teams', $1, 'team-ctx', 'team-ctx') RETURNING id",
        )
        .bind(team)
        .fetch_one(&pool)
        .await
        .unwrap();
        let r = mk_homed_resource(&pool, team_ctx, leaver).await;

        let admin_p = crate::test_support::human_principal_for(&pool, *admin).await;
        crate::services::team_service::remove_member(&pool, &admin_p, team, *leaver)
            .await
            .expect("remove the leaver through the recorded write");

        let moved = reassign_team_resources(&pool, &admin_p, team, *leaver, *steward)
            .await
            .expect("a former member's work is handed off");
        assert_eq!(moved, vec![r]);
        assert_eq!(owner_of(&pool, r).await, *steward);
    }
}
