//! Resource ownership reassignment over `kb_resource_homes`, event-sourced via
//! `writes::reassign_resource_with`. Service-direct (no Backend-trait command),
//! same precedent as `invitation_service` / `team_service`. Authorization
//! precedes every write.
//!
//! Two authorized paths: the current owner may reassign their own resource to
//! anyone (mis-attribution self-fix); a team admin may reassign a resource
//! scoped to a team they manage, to a member of that team (offboarding +
//! admin-assisted mis-attribution). Only **context-homed** resources are
//! reassignable — a cogmap interior is team-resource-derived, not personally
//! owned, and is refused here (and by the `resource_reassign` SQL backstop).

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::team_service::{can_manage, role_on_team};
use temper_core::types::ids::ProfileId;

/// A resource's home owner + the anchor table it's homed under.
struct HomeRow {
    owner: Uuid,
    anchor_table: String,
}

async fn home_of(pool: &PgPool, resource: Uuid) -> ApiResult<HomeRow> {
    sqlx::query_as!(
        HomeRow,
        "SELECT owner_profile_id AS owner, anchor_table \
           FROM kb_resource_homes WHERE resource_id = $1",
        resource,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Is there a team T where caller manages T, `resource` is homed in a context shared
/// to T, and `to` is a member of T? (admin-path reach: from-scope + into-scope).
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
            FROM kb_team_contexts tc
            JOIN kb_teams t
              ON t.id = tc.team_id AND t.is_active
            JOIN kb_resource_homes h
              ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
            JOIN kb_team_members cm
              ON cm.team_id = tc.team_id AND cm.profile_id = $2
                 AND cm.role IN ('owner', 'maintainer')
            JOIN kb_team_members tm
              ON tm.team_id = tc.team_id AND tm.profile_id = $3
            WHERE h.resource_id = $1
        ) AS "exists!: bool"
        "#,
        resource,
        *caller,
        to,
    )
    .fetch_one(pool)
    .await?)
}

/// Reassign a resource's owner to `to_profile_id`. Auth: current owner (any target)
/// OR team-admin over a team the resource is scoped to, to a member of that team.
/// Reassigning to the current owner is an idempotent no-op. Cogmap-homed resources
/// are rejected (map interiors are not personally owned).
pub async fn reassign_resource(
    pool: &PgPool,
    caller: ProfileId,
    resource_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<()> {
    let home = home_of(pool, resource_id).await?;

    // Only context-homed resources are reassignable. The owner path would otherwise
    // let a cogmap-node owner flip it — guard here for BOTH paths (the admin path's
    // reach query already excludes non-context homes structurally).
    if home.anchor_table != "kb_contexts" {
        return Err(ApiError::BadRequest(
            "cannot reassign a cogmap-homed resource; map interiors are not personally owned"
                .to_string(),
        ));
    }

    // Auth before writes: current owner, or an admin with reach over the resource+target.
    let authorized =
        home.owner == *caller || admin_reach(pool, caller, resource_id, to_profile_id).await?;
    if !authorized {
        return Err(ApiError::Forbidden);
    }
    if home.owner == to_profile_id {
        return Ok(()); // idempotent no-op
    }

    // The owner path admits any target; verify it's a real profile so a bad UUID is a
    // clean 400 rather than an FK-violation 500 in the projector. (The admin path's
    // membership join already guarantees existence, but this covers both uniformly.)
    let to_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE id = $1) AS "e!: bool""#,
        to_profile_id,
    )
    .fetch_one(pool)
    .await?;
    if !to_exists {
        return Err(ApiError::BadRequest(
            "target profile does not exist".to_string(),
        ));
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::reassign_resource_with(
        pool,
        temper_substrate::ids::ResourceId::from(resource_id),
        home.owner.into(),
        to_profile_id.into(),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

/// Bulk-reassign, from `from_profile_id` to `to_profile_id`, every resource owned by
/// `from` and homed in a context shared to `team_id`. Auth: caller manages the team AND
/// `to` is a member of it. One transaction; returns the reassigned resource ids.
pub async fn reassign_team_resources(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    from_profile_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<Vec<Uuid>> {
    // A soft-deleted team is inert — it confers no reassignment authority.
    // (`role_on_team` is is_active-blind; team_service gates is_active at each call
    // site, so we do the same here.)
    let team_active = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1 AND is_active) AS "e!: bool""#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    if !team_active {
        return Err(ApiError::Forbidden);
    }

    // Auth before writes: caller manages the team, and `to` is a member of it.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
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

    // Scope read: resources owned by `from` AND homed in a context shared to the team.
    let targets: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT h.resource_id
        FROM kb_team_contexts tc
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        WHERE tc.team_id = $1 AND h.owner_profile_id = $2
        "#,
        team_id,
        from_profile_id,
    )
    .fetch_all(pool)
    .await?;
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut tx = pool.begin().await?;
    for &rid in &targets {
        temper_substrate::writes::reassign_resource_in_tx(
            &mut tx,
            temper_substrate::ids::ResourceId::from(rid),
            from_profile_id.into(),
            to_profile_id.into(),
            emitter,
            temper_substrate::events::EventContext::default(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    tx.commit().await?;
    Ok(targets)
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
    async fn owner_can_reassign_and_visibility_follows(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let ctx = mk_context(&pool, "alice-ctx", alice).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        reassign_resource(&pool, alice, r, *bob)
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
        reassign_resource(&pool, alice, r, *bob).await.unwrap();
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
        reassign_resource(&pool, alice, r, *alice)
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
        let err = reassign_resource(&pool, mallory, r, *bob)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cannot_reassign_cogmap_homed_resource_even_as_owner(pool: PgPool) {
        let alice = mk_profile(&pool, "alice").await;
        let bob = mk_profile(&pool, "bob").await;
        let r = mk_cogmap_homed_resource(&pool, alice).await;
        let err = reassign_resource(&pool, alice, r, *bob).await.unwrap_err();
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
        let ctx = mk_context(&pool, "shared", alice).await;
        share_ctx(&pool, ctx, team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;

        reassign_resource(&pool, admin, r, *steward)
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
        let ctx = mk_context(&pool, "shared", alice).await;
        share_ctx(&pool, ctx, team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        let err = reassign_resource(&pool, admin, r, *outsider)
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
        let err = reassign_resource(&pool, admin, r, *steward)
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
        let err = reassign_resource(&pool, alice, r, ghost).await.unwrap_err();
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
        add_member(&pool, team, steward, "member").await;
        let ctx = mk_context(&pool, "shared", alice).await;
        share_ctx(&pool, ctx, team).await;
        let r = mk_homed_resource(&pool, ctx, alice).await;
        soft_delete_team(&pool, team).await;

        let err = reassign_resource(&pool, admin, r, *steward)
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
        add_member(&pool, team, steward, "member").await;
        let shared = mk_context(&pool, "shared", leaver).await;
        share_ctx(&pool, shared, team).await;
        let r = mk_homed_resource(&pool, shared, leaver).await;
        soft_delete_team(&pool, team).await;

        let err = reassign_team_resources(&pool, admin, team, *leaver, *steward)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Forbidden));
        assert_eq!(owner_of(&pool, r).await, *leaver, "owner unchanged");
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

        let shared = mk_context(&pool, "shared", leaver).await;
        share_ctx(&pool, shared, team).await;
        let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

        let in_scope = mk_homed_resource(&pool, shared, leaver).await; // owned+scoped → moves
        let out_scope = mk_homed_resource(&pool, private, leaver).await; // owned, not scoped → stays
        let not_leaver = mk_homed_resource(&pool, shared, other).await; // scoped, other owner → stays

        let moved = reassign_team_resources(&pool, admin, team, *leaver, *steward)
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
        let err = reassign_team_resources(&pool, stranger, team, *leaver, *steward)
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
        let err = reassign_team_resources(&pool, admin, team, *leaver, *outsider)
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
        let moved = reassign_team_resources(&pool, admin, team, *leaver, *steward)
            .await
            .expect("empty is ok");
        assert!(moved.is_empty());
    }
}
