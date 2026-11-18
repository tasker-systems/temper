use super::*;
use sqlx::PgPool;
use temper_core::types::connection::ProvisionConnectionRequest;
use temper_core::types::ids::ProfileId;
use temper_core::types::subscription::SubscriptionSelector;

const GITHUB_REPO: &str = "acme/temper";

/// Seed a system admin and return its profile id. Follows the connection_service::seed_admin
/// pattern: gating-team owner + approved standing + governance grant (the D11 model).
async fn seed_admin(pool: &PgPool) -> ProfileId {
    let id = Uuid::now_v7();
    let handle = format!("admin-{id}");
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
        id,
        &handle,
        &handle,
    )
    .execute(pool)
    .await
    .expect("seed admin profile");

    // The caller must carry its `<handle>@web` emitter — provision via the production path.
    let mut conn = pool.acquire().await.expect("acquire");
    crate::services::profile_service::provision_profile_entities(&mut conn, id, &handle)
        .await
        .expect("provision caller emitters");
    drop(conn);

    // gating team + owner (is_system_admin reads governance, but the gating team still
    // needs to exist and the owner row is what the existing fixture pattern writes).
    let team: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("gating team");

    sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
        .execute(pool)
        .await
        .expect("configure gating team");

    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         VALUES ($1, $2, 'owner'::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        team,
        id,
    )
    .execute(pool)
    .await
    .expect("join gating team as owner");

    // What actually confers admin-ness: approved standing + governance grant.
    crate::test_support::approved_admin(pool, id).await;

    ProfileId::from(id)
}

/// Seed a team with the given owner and return its id.
async fn seed_team(pool: &PgPool, owner: ProfileId) -> Uuid {
    let team_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO kb_teams (id, slug, name)
               VALUES ($1, $2, $3)"#,
        team_id,
        format!("team-{team_id}"),
        format!("Team {team_id}"),
    )
    .execute(pool)
    .await
    .expect("seed team");
    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'owner'::team_role)"#,
        team_id,
        *owner,
    )
    .execute(pool)
    .await
    .expect("seed team owner");
    team_id
}

/// Seed an approved, non-admin profile with its emitters. Used wherever a test needs a
/// principal who is admitted to the instance but holds no authority over the subject.
async fn seed_plain_profile(pool: &PgPool, label: &str) -> ProfileId {
    let id = Uuid::now_v7();
    let handle = format!("{label}-{id}");
    crate::test_support::profile::insert_with_id(pool, id, &handle).await;
    let mut acquired = pool.acquire().await.expect("acquire");
    crate::services::profile_service::provision_profile_entities(&mut acquired, id, &handle)
        .await
        .expect("provision emitters");
    drop(acquired);
    crate::test_support::approve(pool, id).await;
    ProfileId::from(id)
}

/// Seed a second team and add `member` as a maintainer (manage-capable but not owner).
async fn seed_team_with_maintainer(pool: &PgPool, maintainer: ProfileId) -> Uuid {
    let team_id = Uuid::now_v7();
    let slug = format!("team-{team_id}");
    sqlx::query!(
        r#"INSERT INTO kb_teams (id, slug, name)
               VALUES ($1, $2, $3)"#,
        team_id,
        &slug,
        &slug,
    )
    .execute(pool)
    .await
    .expect("seed team");
    // The admin is owner (so the team has one), and `maintainer` is maintainer.
    let admin = seed_admin(pool).await;
    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'owner'::team_role)"#,
        team_id,
        *admin,
    )
    .execute(pool)
    .await
    .expect("seed team owner");
    sqlx::query!(
        r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'maintainer'::team_role)"#,
        team_id,
        *maintainer,
    )
    .execute(pool)
    .await
    .expect("seed team maintainer");
    team_id
}

/// Seed a context owned by `team_id` and return its id.
async fn seed_context_owned_by_team(pool: &PgPool, team_id: Uuid) -> Uuid {
    let context_id = Uuid::now_v7();
    let slug = format!("ctx-{context_id}");
    sqlx::query!(
        r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
               VALUES ($1, 'kb_teams', $2, $3, $4)"#,
        context_id,
        team_id,
        &slug,
        &slug,
    )
    .execute(pool)
    .await
    .expect("seed context");
    // Link the context to the team via kb_team_contexts.
    sqlx::query!(
        r#"INSERT INTO kb_team_contexts (context_id, team_id)
               VALUES ($1, $2)"#,
        context_id,
        team_id,
    )
    .execute(pool)
    .await
    .expect("seed team_contexts");
    context_id
}

/// Seed a cogmap and link it to `team_id` via kb_team_cogmaps; return the cogmap id.
/// Creating a cogmap requires a telos_resource, which requires a resource row — `kb_resources`
/// has `title` + `origin_uri` (no context_id, no slug).
async fn seed_cogmap_linked_to_team(pool: &PgPool, team_id: Uuid) -> Uuid {
    let _context_id = seed_context_owned_by_team(pool, team_id).await;
    // kb_resources has title + origin_uri (NOT context_id/slug). Let the DB generate the id
    // and capture it via RETURNING.
    let resource_id: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
        "telos",
        "",
    )
    .fetch_one(pool)
    .await
    .expect("seed resource for cogmap telos");

    let cogmap_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO kb_cogmaps (id, name, telos_resource_id)
               VALUES ($1, $2, $3)"#,
        cogmap_id,
        format!("Cogmap {cogmap_id}"),
        resource_id,
    )
    .execute(pool)
    .await
    .expect("seed cogmap");

    sqlx::query!(
        r#"INSERT INTO kb_team_cogmaps (cogmap_id, team_id)
               VALUES ($1, $2)"#,
        cogmap_id,
        team_id,
    )
    .execute(pool)
    .await
    .expect("seed team_cogmaps");
    cogmap_id
}

/// Seed a connection owned by `team_id` and return its id. Uses the connection_service
/// provision path so the profile, entity, and home context are all created correctly.
async fn seed_connection(pool: &PgPool, owner_team_id: Option<Uuid>, caller: ProfileId) -> Uuid {
    let req = ProvisionConnectionRequest {
        provider: "github".into(),
        name: format!("test-conn-{}", Uuid::now_v7()),
        owner_team_id,
        reach_granularity: None,
        reach_covers: None,
    };
    let conn = crate::services::connection_service::provision(
        pool,
        &crate::test_support::human_principal_for(pool, caller.uuid()).await,
        &req,
    )
    .await
    .expect("seed connection");
    conn.id
}

/// Grant read-reach on `connection_id` to `team_id`. Writes the kb_access_grants row B2 made
/// possible. No affirmation needed — the connection declares no reach.
async fn grant_reach(pool: &PgPool, caller: ProfileId, connection_id: Uuid, team_id: Uuid) {
    crate::services::connection_service::grant_reach(
        pool,
        &crate::test_support::human_principal_for(pool, caller.uuid()).await,
        connection_id,
        team_id,
        None,
    )
    .await
    .expect("grant reach");
}

fn github_repo_selector() -> SubscriptionSelector {
    SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request.merged".into()],
    }
}

fn req(
    subscriber_table: &str,
    subscriber_id: Uuid,
    authoring_team_id: Uuid,
    connection_id: Uuid,
) -> CreateSubscriptionRequest {
    CreateSubscriptionRequest {
        subscriber_table: subscriber_table.into(),
        subscriber_id,
        authoring_team_id,
        connection_id,
        selector: github_repo_selector(),
    }
}

/// Leg 3 rests on the admin regime even when leg 1 did not: an admin who also manages the
/// authoring team passes leg 1 on the team arm, and leg 3 for a map bound to the gating team
/// (admin-only) is `is_system_admin`. Their demotion in flight blocks the insert's transaction
/// at the governance lock; once it commits, leg 3 asked again on that transaction refuses, and
/// no subscription is written. Fails if leg 3 is not re-run on the transaction under the lock:
/// the insert then delivers onto an admin-only map the caller no longer authors.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_demotion_in_flight_refuses_a_team_managers_subscription_onto_an_admin_only_map(
    pool: PgPool,
) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let map = seed_cogmap_linked_to_team(&pool, team).await;
    sqlx::query(
        "INSERT INTO kb_team_cogmaps (cogmap_id, team_id) \
         SELECT $1, id FROM kb_teams WHERE slug = 'temper-system'",
    )
    .bind(map)
    .execute(&pool)
    .await
    .expect("bind the map to the gating team: admin-only");

    let mut holder = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let _: bool = sqlx::query_scalar("SELECT principal_governance_set($1, false, NULL, 'test')")
        .bind(admin.uuid())
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    let (p, by) = (
        pool.clone(),
        crate::test_support::human_principal_for(&pool, admin.uuid()).await,
    );
    let contender =
        tokio::spawn(async move { create(&p, &by, &req("kb_cogmaps", map, team, conn)).await });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)) \
                               AND query ILIKE '%FROM kb_principal_governance%FOR KEY SHARE%')",
        )
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
        if waiting {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the create never blocked at the governance lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    holder.commit().await.unwrap();
    let err = contender.await.unwrap().unwrap_err();
    assert!(matches!(err, ApiError::Forbidden), "{err:?}");
    let written: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_subscriptions WHERE subscriber_id = $1")
            .bind(map)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(written, 0, "a refused create subscribed the admin-only map");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_and_revoke_a_team_subscription_round_trips(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // A team subscribes for itself: subscriber_table = kb_teams, subscriber_id = team,
    // authoring_team_id = team.
    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("create subscription");

    assert_eq!(created.subscriber_table, "kb_teams");
    assert_eq!(created.subscriber_id, team);
    assert_eq!(created.authoring_team_id, team);
    assert_eq!(created.connection_id, conn);
    assert!(created.revoked_at.is_none());

    // Revoke.
    let revoked = revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        created.id,
    )
    .await
    .expect("revoke");
    assert!(revoked.revoked_at.is_some());
    assert_eq!(revoked.revoked_by_profile_id, Some(*admin));
}

/// BITE: create's team-manager arm is decided again under the authoring team's lock. An admin
/// removes the team's maintainer, uncommitted; the maintainer's create blocks on the team row;
/// the removal commits. The maintainer is refused as a caller who manages nothing there is
/// (`403`), and no subscription is written. Before the fix the pool check's verdict stood.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_manager_removed_while_create_waits_on_the_team_lock_subscribes_nothing(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let maintainer = seed_plain_profile(&pool, "removed-maintainer").await;
    let stranger = seed_plain_profile(&pool, "stranger").await;
    let team = seed_team_with_maintainer(&pool, maintainer).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let stranger_err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect_err("a stranger to the team subscribes nothing");

    let holder =
        crate::test_support::hold_member_removal(&pool, admin.uuid(), team, maintainer.uuid())
            .await;
    let (p, by) = (
        pool.clone(),
        crate::test_support::human_principal_for(&pool, maintainer.uuid()).await,
    );
    let contender =
        tokio::spawn(async move { create(&p, &by, &req("kb_teams", team, team, conn)).await });
    crate::test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let err = contender
        .await
        .unwrap()
        .expect_err("a manager removed under the lock subscribes nothing");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    assert_eq!(
        format!("{err:?}"),
        format!("{stranger_err:?}"),
        "the removed manager is told what a stranger is told"
    );
    let written: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_subscriptions WHERE connection_id = $1")
            .bind(conn)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(written, 0, "a refused create wrote a subscription");
}

/// The admin arm of subscription create and revoke is asked again on the write's transaction,
/// under the governance lock. The admin's demotion is left open; each door blocks at the
/// governance re-check (the blocked statement is matched), then is refused once it commits,
/// having written nothing. Without the re-check neither door blocks, and both wrote. The team is
/// someone else's, so the demoted admin holds no other arm.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_demotion_in_flight_refuses_create_and_revoke(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let owner = seed_admin(&pool).await;
    let team = seed_team(&pool, owner).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let existing = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("seed subscription");
    sqlx::query("UPDATE kb_subscriptions SET selector = jsonb_set(selector, '{repo}', '\"acme/seeded\"') WHERE id = $1")
        .bind(existing.id)
        .execute(&pool)
        .await
        .unwrap();

    async fn in_flight<T: std::fmt::Debug + Send + 'static>(
        pool: &PgPool,
        admin: ProfileId,
        door: impl std::future::Future<Output = ApiResult<T>> + Send + 'static,
    ) -> ApiError {
        let mut holder = pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *holder)
            .await
            .unwrap();
        let _: bool =
            sqlx::query_scalar("SELECT principal_governance_set($1, false, NULL, 'test')")
                .bind(admin.uuid())
                .fetch_one(&mut *holder)
                .await
                .unwrap();
        let contender = tokio::spawn(door);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                                 WHERE datname = current_database() AND wait_event_type = 'Lock' \
                                   AND $1 = ANY (pg_blocking_pids(pid)) \
                                   AND query ILIKE '%FROM kb_principal_governance%FOR KEY SHARE%')",
            )
            .bind(pid)
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the door never blocked at the governance re-check"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        holder.commit().await.unwrap();
        let err = contender.await.unwrap().unwrap_err();
        crate::test_support::grant_governance(pool, admin.uuid()).await;
        err
    }

    let (p, by) = (
        pool.clone(),
        crate::test_support::human_principal_for(&pool, admin.uuid()).await,
    );
    let err = in_flight(&pool, admin, async move {
        create(&p, &by, &req("kb_teams", team, team, conn)).await
    })
    .await;
    assert!(matches!(err, ApiError::Forbidden), "create: {err:?}");
    let live: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_subscriptions WHERE connection_id = $1")
            .bind(conn)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(live, 1, "a refused create wrote a subscription");

    let (p, by) = (
        pool.clone(),
        crate::test_support::human_principal_for(&pool, admin.uuid()).await,
    );
    let id = existing.id;
    let err = in_flight(&pool, admin, async move { revoke(&p, &by, id).await }).await;
    assert!(
        matches!(&err, ApiError::NotFound(why) if why == SUBSCRIPTION_REFUSAL),
        "revoke: {err:?}"
    );
    assert!(
        get(&pool, id).await.unwrap().revoked_at.is_none(),
        "a refused revoke revoked"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_refused_without_reach_grant(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    // NOTE: no grant_reach — the team does NOT hold a reach grant on the connection.

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect_err("should be forbidden");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_refused_without_manage_role(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // A stranger with no role on the team.
    let stranger = seed_plain_profile(&pool, "stranger").await;

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect_err("should be forbidden");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

/// A system admin may act on a subscription whose authoring team it does not manage.
///
/// `list` has always admitted an admin; `create`/`get`/`revoke` did not, so an admin could see
/// every subscription in the instance and revoke none of them. The module doc has described the
/// gate as *"composed with the system-admin path at the authority layer"* since this file was
/// written — there was no authority layer, and no admin leg.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn system_admin_may_act_on_a_team_it_does_not_manage(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let outsider = seed_plain_profile(&pool, "team-owner").await;
    let team = seed_team(&pool, outsider).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // The precondition this test turns on: the admin holds NO role on the authoring team. If
    // the fixture ever grants one, the test would pass for a reason that has nothing to do
    // with the admin leg.
    assert!(
        crate::services::team_service::role_on_team(&pool, team, admin)
            .await
            .expect("role lookup")
            .is_none(),
        "fixture invalid: the admin manages the authoring team, so this test cannot \
         distinguish the admin leg from the manage leg"
    );

    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("admin creates on a team it does not manage");
    let read = get_for_caller(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        created.id,
    )
    .await
    .expect("admin reads on a team it does not manage");
    assert_eq!(read.id, created.id);
    let revoked = revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        created.id,
    )
    .await
    .expect("admin revokes on a team it does not manage");
    assert!(revoked.revoked_at.is_some());
    assert_eq!(revoked.revoked_by_profile_id, Some(*admin));
}

/// The admin leg widens WHO may declare for a team; it does not exempt the team from leg 2.
///
/// `create_refused_without_reach_grant` looks like it covers this and does not: its caller is
/// the team's own owner, so it proves leg 2 binds a *manager*. This is the admin arm of the
/// same claim, and it is what makes the comment at the leg-2 call site true by test rather
/// than by assertion.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_admin_leg_does_not_exempt_the_authoring_team_from_reach(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let outsider = seed_plain_profile(&pool, "team-owner").await;
    let team = seed_team(&pool, outsider).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    // NOTE: no grant_reach — the authoring team holds no reach on the connection.

    assert!(
        crate::services::team_service::role_on_team(&pool, team, admin)
            .await
            .expect("role lookup")
            .is_none(),
        "fixture invalid: the admin manages the authoring team, so leg 1 would admit it as a \
         manager and this test would not be exercising the admin leg"
    );

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect_err("an admin still needs the team to hold reach");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

/// The other direction, on the same three acts. Widening a gate is only safe if the arm that
/// was already denied is still denied — an admin leg that admits everyone is not an admin leg.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_non_admin_who_does_not_manage_is_still_refused_all_three_acts(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let outsider = seed_plain_profile(&pool, "team-owner").await;
    let team = seed_team(&pool, outsider).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let stranger = seed_plain_profile(&pool, "stranger").await;

    // create
    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect_err("stranger must not create");
    assert!(matches!(err, ApiError::Forbidden), "create: got {err:?}");

    // A real row to aim get/revoke at, authored by someone who may.
    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, outsider.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("the team owner may create");

    let err = get_for_caller(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        created.id,
    )
    .await
    .expect_err("stranger must not read");
    assert!(
        matches!(&err, ApiError::NotFound(m) if m == SUBSCRIPTION_REFUSAL),
        "get: a subscription the caller cannot manage reads as missing, got {err:?}"
    );

    let err = revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        created.id,
    )
    .await
    .expect_err("stranger must not revoke");
    assert!(
        matches!(&err, ApiError::NotFound(m) if m == SUBSCRIPTION_REFUSAL),
        "revoke: a subscription the caller cannot manage reads as missing, got {err:?}"
    );

    // And it is the same refusal a missing id gets, byte for byte: the existence oracle, closed.
    let prober = crate::test_support::human_principal_for(&pool, stranger.uuid()).await;
    let missing = uuid::Uuid::now_v7();
    assert_eq!(
        get_for_caller(&pool, &prober, created.id)
            .await
            .expect_err("denied")
            .to_string(),
        get_for_caller(&pool, &prober, missing)
            .await
            .expect_err("missing")
            .to_string(),
        "get: a denied id must refuse like a missing one"
    );
    assert_eq!(
        revoke(&pool, &prober, created.id)
            .await
            .expect_err("denied")
            .to_string(),
        revoke(&pool, &prober, missing)
            .await
            .expect_err("missing")
            .to_string(),
        "revoke: a denied id must refuse like a missing one"
    );

    // And the refusal was real, not a silent no-op: the row is still live.
    let still = get(&pool, created.id).await.expect("row still readable");
    assert!(
        still.revoked_at.is_none(),
        "the refused revoke must not have taken effect"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn cross_team_subscription_legal_when_reach_granted(pool: PgPool) {
    // The platform team owns the connection; a different team (the subscriber) is granted
    // reach, then subscribes. The connection's owning team is NOT consulted for the
    // subscription — owner ≠ reach.
    let admin = seed_admin(&pool).await;
    let platform_team = seed_team(&pool, admin).await;
    let subscriber_team = seed_team_with_maintainer(&pool, admin).await;
    let conn = seed_connection(&pool, Some(platform_team), admin).await;

    // The platform team grants reach to the subscriber team.
    grant_reach(&pool, admin, conn, subscriber_team).await;

    // A maintainer of the subscriber team creates the subscription.
    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", subscriber_team, subscriber_team, conn),
    )
    .await
    .expect("cross-team subscription should succeed");

    assert_eq!(created.authoring_team_id, subscriber_team);
    assert_eq!(created.connection_id, conn);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn subscription_against_ledger_only_connection_is_legal(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    // A connection with no webhook_events and no tool_manifest — ledger-only (in fact,
    // born needs_credential and both tiers empty). A subscription against it is legal and
    // stored — it says "inert for judgment."
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("ledger-only subscription should be legal");

    assert_eq!(created.connection_id, conn);
    assert!(created.revoked_at.is_none());
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoke_but_never_delete(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("create");

    let revoked = revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        created.id,
    )
    .await
    .expect("revoke");
    assert!(revoked.revoked_at.is_some());

    // The row is still there — never deleted.
    let still_there = get(&pool, created.id)
        .await
        .expect("row should still exist");
    assert_eq!(still_there.id, created.id);
    assert!(still_there.revoked_at.is_some());

    // Double revoke is a no-op returning the existing row (first revoker is the truth).
    let double = revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        created.id,
    )
    .await
    .expect("double revoke");
    assert_eq!(double.revoked_by_profile_id, revoked.revoked_by_profile_id);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn unique_constraint_rejects_duplicate_selector(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let _first = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("first create");

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect_err("duplicate should conflict");
    assert!(matches!(err, ApiError::Conflict(_)), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn distinct_selectors_against_same_connection_are_distinct(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let first = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team, team, conn),
    )
    .await
    .expect("first create");

    // Same connection, same team, different selector (different repo).
    let second_req = CreateSubscriptionRequest {
        subscriber_table: "kb_teams".into(),
        subscriber_id: team,
        authoring_team_id: team,
        connection_id: conn,
        selector: SubscriptionSelector::GitHubRepository {
            repo: "acme/other-repo".into(),
            event_types: vec![],
        },
    };
    let second = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &second_req,
    )
    .await
    .expect("second create with different selector");

    assert_ne!(first.id, second.id);
    assert_eq!(first.connection_id, second.connection_id);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn kb_teams_subscriber_requires_authoring_team_equals_subscriber(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team_a = seed_team(&pool, admin).await;
    let team_b = seed_team_with_maintainer(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team_a), admin).await;
    grant_reach(&pool, admin, conn, team_b).await;

    // Try to subscribe as team_b but with authoring_team_id = team_a — should be a 400.
    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team_b, team_a, conn),
    )
    .await
    .expect_err("authoring_team != subscriber for kb_teams should be 400");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn kb_contexts_subscriber_validates_team_link(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let context_id = seed_context_owned_by_team(&pool, team).await;

    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_contexts", context_id, team, conn),
    )
    .await
    .expect("context subscription should succeed");

    assert_eq!(created.subscriber_table, "kb_contexts");
    assert_eq!(created.subscriber_id, context_id);
    assert_eq!(created.authoring_team_id, team);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn kb_contexts_subscriber_refused_when_team_not_linked(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let other_team = seed_team_with_maintainer(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, other_team).await;

    // Create a context owned by `team`, but try to subscribe with `other_team` as the
    // authoring team — other_team is NOT linked to this context.
    let context_id = seed_context_owned_by_team(&pool, team).await;

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_contexts", context_id, other_team, conn),
    )
    .await
    .expect_err("unlinked team should be 400");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

/// Give `profile` an explicit write grant on a context or map (`subject_table`).
async fn grant_write(pool: &PgPool, subject_table: &str, subject: Uuid, profile: ProfileId) {
    sqlx::query!(
        r#"INSERT INTO kb_access_grants (subject_table, subject_id, principal_table,
                                            principal_id, can_read, can_write, granted_by_profile_id)
               VALUES ($1, $2, 'kb_profiles', $3, true, true, $3)"#,
        subject_table,
        subject,
        *profile,
    )
    .execute(pool)
    .await
    .expect("grant write");
}

/// The subscriber leg binds a system admin as it binds everyone: a map's authorship is its write
/// grant, and the admin holds none until one is given. Then the link is all that is checked.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn kb_cogmaps_subscriber_validates_team_link(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let cogmap_id = seed_cogmap_linked_to_team(&pool, team).await;

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_cogmaps", cogmap_id, team, conn),
    )
    .await
    .expect_err("a system admin who does not author the map is refused");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");

    grant_write(&pool, "kb_cogmaps", cogmap_id, admin).await;
    let created = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_cogmaps", cogmap_id, team, conn),
    )
    .await
    .expect("cogmap subscription should succeed");

    assert_eq!(created.subscriber_table, "kb_cogmaps");
    assert_eq!(created.subscriber_id, cogmap_id);
    assert_eq!(created.authoring_team_id, team);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn list_scopes_to_teams_the_caller_manages(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team_a = seed_team(&pool, admin).await;
    let team_b = seed_team_with_maintainer(&pool, admin).await;
    let conn_a = seed_connection(&pool, Some(team_a), admin).await;
    let conn_b = seed_connection(&pool, Some(team_b), admin).await;
    grant_reach(&pool, admin, conn_a, team_a).await;
    grant_reach(&pool, admin, conn_b, team_b).await;

    let _sub_a = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team_a, team_a, conn_a),
    )
    .await
    .expect("sub a");
    let _sub_b = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &req("kb_teams", team_b, team_b, conn_b),
    )
    .await
    .expect("sub b");

    // admin is system admin → sees all.
    let all = list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        false,
        None,
    )
    .await
    .expect("admin list");
    assert_eq!(all.len(), 2);

    // A stranger with no role on either team sees none. Seed a stranger with system access
    // (so they can call list at all) but no team role.
    let stranger = ProfileId::from(Uuid::now_v7());
    let stranger_handle = format!("stranger-{}", *stranger);
    sqlx::query!(
        r#"INSERT INTO kb_profiles (id, handle, display_name)
               VALUES ($1, $2, $3)"#,
        *stranger,
        &stranger_handle,
        &stranger_handle,
    )
    .execute(&pool)
    .await
    .expect("seed stranger");
    let mut acquired = pool.acquire().await.expect("acquire");
    crate::services::profile_service::provision_profile_entities(
        &mut acquired,
        *stranger,
        &stranger_handle,
    )
    .await
    .expect("provision stranger emitters");
    drop(acquired);
    crate::test_support::approve(&pool, *stranger).await;

    let none = list(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        false,
        None,
    )
    .await
    .expect("stranger list");
    assert!(none.is_empty(), "stranger should see no subscriptions");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn bad_subscriber_table_is_bad_request(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let err = create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &CreateSubscriptionRequest {
            subscriber_table: "kb_resources".into(), // not admissible
            subscriber_id: team,
            authoring_team_id: team,
            connection_id: conn,
            selector: github_repo_selector(),
        },
    )
    .await
    .expect_err("bad subscriber_table should be 400");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

/// The subscriber must be authored by the caller, not merely linked to their team. A context owned
/// elsewhere and shared into the authoring team for reading is linked, and a maintainer of that
/// team may not stamp events onto it. A write grant on the context, which is what authoring it
/// means for someone outside its owning team, admits the same call. The bite: delete the
/// subscriber-authorship leg in `create`, and the first create succeeds.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_context_shared_in_for_reading_is_not_subscribable(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let maintainer = seed_plain_profile(&pool, "maintainer").await;
    let team = seed_team_with_maintainer(&pool, maintainer).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // Owned by another team the maintainer is not in, then shared into `team`.
    let elsewhere_owner = seed_plain_profile(&pool, "elsewhere").await;
    let elsewhere = seed_team(&pool, elsewhere_owner).await;
    let context_id = seed_context_owned_by_team(&pool, elsewhere).await;
    sqlx::query!(
        "INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)",
        context_id,
        team,
    )
    .execute(&pool)
    .await
    .expect("share the context into the team");

    let as_maintainer = crate::test_support::human_principal_for(&pool, maintainer.uuid()).await;
    let err = create(
        &pool,
        &as_maintainer,
        &req("kb_contexts", context_id, team, conn),
    )
    .await
    .expect_err("a context the caller only reads is refused");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");

    grant_write(&pool, "kb_contexts", context_id, maintainer).await;
    create(
        &pool,
        &as_maintainer,
        &req("kb_contexts", context_id, team, conn),
    )
    .await
    .expect("with write on the context, the same subscription is admitted");
}

/// The map arm of the same leg: a map bound to the team is linked, and its subscriber still needs
/// authorship, which for a map is an explicit write grant. The bite: as above.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_map_bound_to_the_team_needs_its_author_to_subscribe(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let maintainer = seed_plain_profile(&pool, "map-maintainer").await;
    let team = seed_team_with_maintainer(&pool, maintainer).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let cogmap_id = seed_cogmap_linked_to_team(&pool, team).await;

    let as_maintainer = crate::test_support::human_principal_for(&pool, maintainer.uuid()).await;
    let err = create(
        &pool,
        &as_maintainer,
        &req("kb_cogmaps", cogmap_id, team, conn),
    )
    .await
    .expect_err("a map the caller does not author is refused");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");

    grant_write(&pool, "kb_cogmaps", cogmap_id, maintainer).await;
    create(
        &pool,
        &as_maintainer,
        &req("kb_cogmaps", cogmap_id, team, conn),
    )
    .await
    .expect("the map's author may subscribe it");
}
