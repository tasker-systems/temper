#![cfg(feature = "test-db")]

mod common;

use temper_core::types::admin::UpdateSettingsRequest;
use temper_services::services::access_service;
use temper_services::test_support;
use uuid::Uuid;

/// A sealed `SystemAdmin` proof for the mechanics tests below. They exercise service behavior
/// (coalesce, lockout guards, promotion, auto-join enrollment), not the authz gate — but the acts now
/// require the proof, so mint a real one (admin-authz enclosure, spec §3).
async fn admin_proof(pool: &sqlx::PgPool) -> temper_services::auth::SystemAdmin {
    test_support::system_admin_proof(pool).await
}

/// Seed the singleton settings row to a known baseline (the seed migration
/// inserts `id=1` already, but be explicit so the test is self-contained).
async fn reset_settings(pool: &sqlx::PgPool) {
    sqlx::query(
        "UPDATE kb_system_settings \
         SET gating_team_slug=NULL, instance_name=NULL, \
             terms_version=NULL, terms_resource_uri=NULL WHERE id=1",
    )
    .execute(pool)
    .await
    .expect("reset settings");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_partial_coalesces(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;

    let req = UpdateSettingsRequest {
        instance_name: Some("Acme Temper".to_owned()),
        ..Default::default()
    };
    let updated = access_service::update_system_settings(&pool, &admin, &req)
        .await
        .expect("update");

    assert_eq!(updated.instance_name.as_deref(), Some("Acme Temper"));
    // (the COALESCE "untouched field preserved" check previously asserted on `access_mode`, which
    // Phase 2 removed from the settings wire type; instance_name above carries the same guarantee.)
}

// The `access_mode` control is retired (spec §14 / D18): `update_system_settings` no longer accepts
// it, so the old "rejects unknown access_mode" and "invite_only requires a gating team" tests are
// gone with the behaviors they pinned. The lockout they guarded can no longer happen — Task 7's
// repoint made `has_system_access` read standing, not gating-team membership. The gating-team-exists
// guard survives, decoupled from the mode; it is exercised by
// `update_settings_rejects_nonexistent_gating_team` below.

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_defaults_to_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    // Configure a gating team that exists.
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("set gating");

    let profile = common::fixtures::create_test_profile(&pool, "promotee@test.example.com").await;

    let row = access_service::promote_admin(&pool, &admin, profile, None)
        .await
        .expect("promote");

    assert_eq!(row.team_id, team_id);
    assert_eq!(row.profile_id, profile);
    assert!(matches!(
        row.role,
        temper_core::types::team::TeamRole::Owner
    ));

    // is_system_admin now true for the promotee.
    let is_admin =
        access_service::is_system_admin(&pool, temper_core::types::ids::ProfileId::from(profile))
            .await
            .expect("check");
    assert!(is_admin);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_without_gating_or_team_is_bad_request(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // gating_team_slug NULL, no --team
    let admin = admin_proof(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "x@test.example.com").await;
    let err = access_service::promote_admin(&pool, &admin, profile, None)
        .await
        .expect_err("no target team");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_rejects_nonexistent_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // no team named "does-not-exist"
    let admin = admin_proof(&pool).await;

    // The gating-team-exists guard survives the access_mode retirement, decoupled from any mode:
    // pointing the gating slug at a nonexistent team would silently break admin resolution
    // (`is_system_admin` reads governance keyed on that team's ownership).
    let req = UpdateSettingsRequest {
        gating_team_slug: Some("does-not-exist".to_owned()),
        ..Default::default()
    };
    let err = access_service::update_system_settings(&pool, &admin, &req)
        .await
        .expect_err("a nonexistent gating team should be rejected");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_rejects_nonexistent_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "p@test.example.com").await;
    // Pass a random team_id that does not exist in kb_teams.
    let bad_team_id = Uuid::new_v4();
    let err = access_service::promote_admin(&pool, &admin, profile, Some(bad_team_id))
        .await
        .expect_err("explicit nonexistent team should be rejected");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_rejects_nonexistent_profile(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    // Configure a real gating team so the None branch resolves.
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(&pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("set gating");

    // Pass a random profile_id that does not exist in kb_profiles.
    let bad_profile_id = Uuid::new_v4();
    let err = access_service::promote_admin(&pool, &admin, bad_profile_id, None)
        .await
        .expect_err("nonexistent profile should be rejected");
    assert!(matches!(
        err,
        temper_services::error::ApiError::BadRequest(_)
    ));
}

// ─── a soft-deleted gating team confers nothing ──────────────────────────────
//
// `20260703000001_team_metadata_soft_delete.sql` declares in its header that "membership in a
// soft-deleted team confers nothing anywhere". `promote_admin`'s default branch and
// `create_join_request` both resolve the gating team by slug in Rust rather than through the SQL
// chokepoints, so each has to state the filter itself — otherwise a promotion mints an `owner` row
// on a dead team, and a join request is filed against (and later approved into) one.
//
// Each keeps its own incumbent refusal, and a soft-deleted gating team is indistinguishable from a
// slug naming no team at all: from the caller's side both are the settings row pointing at nothing.

/// Soft-delete a team the way `DELETE /api/teams/{id}` does — a bare `is_active` flip that leaves
/// every membership row intact, so these tests cannot pass because something also tidied up.
async fn soft_delete_team(pool: &sqlx::PgPool, team_id: Uuid) {
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(team_id)
        .execute(pool)
        .await
        .expect("soft-delete team");
}

/// Insert a gating team and point `kb_system_settings` at it. Raw SQL, not
/// `update_system_settings`, so the slug can later be pointed at an absent team for the
/// "indistinguishable from never existed" half (that service call guards existence).
async fn configure_gating(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    let team_id: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .expect("gating team");
    point_gating_at(pool, slug).await;
    team_id
}

/// Point the settings row's `gating_team_slug` at `slug`, whether or not a team carries it.
async fn point_gating_at(pool: &sqlx::PgPool, slug: &str) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = $1 WHERE id = 1")
        .bind(slug)
        .execute(pool)
        .await
        .expect("set gating slug");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_refuses_a_soft_deleted_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let admin = admin_proof(&pool).await;
    let team_id = configure_gating(&pool, "sd-gating-promote").await;

    // (a) While the gating team is ACTIVE the promotion lands — the before half, so a regression
    //     cannot make this test pass by breaking promotion outright.
    let first =
        common::fixtures::create_test_profile(&pool, "sd-promotee-1@test.example.com").await;
    let row = access_service::promote_admin(&pool, &admin, first, None)
        .await
        .expect("promotion onto an ACTIVE gating team should land");
    assert_eq!(row.team_id, team_id);

    // (b) Soft-delete it. Every membership row survives, so the refusal can only be `is_active`.
    soft_delete_team(&pool, team_id).await;

    let second =
        common::fixtures::create_test_profile(&pool, "sd-promotee-2@test.example.com").await;
    let err = access_service::promote_admin(&pool, &admin, second, None)
        .await
        .expect_err("a soft-deleted gating team is no promotion target");
    let temper_services::error::ApiError::BadRequest(msg) = err else {
        panic!("expected BadRequest for a soft-deleted gating team, got {err:?}");
    };

    // Indistinguishable from a gating slug that names no team at all.
    point_gating_at(&pool, "sd-gating-absent").await;
    let third =
        common::fixtures::create_test_profile(&pool, "sd-promotee-3@test.example.com").await;
    let absent_err = access_service::promote_admin(&pool, &admin, third, None)
        .await
        .expect_err("a gating slug naming no team is no promotion target either");
    let temper_services::error::ApiError::BadRequest(absent_msg) = absent_err else {
        panic!("expected BadRequest for an absent gating team, got {absent_err:?}");
    };
    assert_eq!(
        msg.replace("sd-gating-promote", "<slug>"),
        absent_msg.replace("sd-gating-absent", "<slug>"),
        "a soft-deleted gating team must refuse in the same words as one that never existed"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_refuses_a_soft_deleted_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    let team_id = configure_gating(&pool, "sd-gating-join").await;

    // `create_test_profile` grants `approved` standing so its callers clear the front door, but
    // `Request` is legal only from `denied` — so put each requester back where a provisioned
    // principal starts. `denied` is a ROW, not an absent one: no standing at all is its own refusal
    // ("not legal for a principal with no standing"), which would stop these arms in the standing
    // machine before they ever reach the gating-team lookup this test is about.
    async fn denied_profile(pool: &sqlx::PgPool, email: &str) -> Uuid {
        let profile = common::fixtures::create_test_profile(pool, email).await;
        sqlx::query(
            "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'denied')
             ON CONFLICT (profile_id) DO UPDATE SET state = 'denied', updated = now()",
        )
        .bind(profile)
        .execute(pool)
        .await
        .expect("set standing to denied");
        profile
    }

    let request_for = |profile: Uuid| access_service::CreateJoinRequestParams {
        profile_id: temper_core::types::ids::ProfileId::from(profile),
        message: None,
        source: "test".to_owned(),
        accepted_terms_version: None,
    };

    // (a) ACTIVE gating team: the request is filed against it.
    let first = denied_profile(&pool, "sd-joiner-1@test.example.com").await;
    let req = access_service::create_join_request(&pool, request_for(first))
        .await
        .expect("a request against an ACTIVE gating team should be filed");
    assert_eq!(req.team_id, team_id);

    // (b) Soft-delete it. The refusal must arrive before the standing write, so the requester is
    //     left where they started rather than stranded in `requested` with no request row.
    soft_delete_team(&pool, team_id).await;

    let second = denied_profile(&pool, "sd-joiner-2@test.example.com").await;
    let err = access_service::create_join_request(&pool, request_for(second))
        .await
        .expect_err("a soft-deleted gating team must accept no join request");
    let temper_services::error::ApiError::Internal(msg) = err else {
        panic!("expected Internal for a soft-deleted gating team, got {err:?}");
    };
    let filed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kb_join_requests WHERE requesting_profile_id = $1)",
    )
    .bind(second)
    .fetch_one(&pool)
    .await
    .expect("check");
    assert!(!filed, "the refusal must precede the request row");

    // Indistinguishable from a gating slug that names no team at all.
    point_gating_at(&pool, "sd-gating-absent").await;
    let third = denied_profile(&pool, "sd-joiner-3@test.example.com").await;
    let absent_err = access_service::create_join_request(&pool, request_for(third))
        .await
        .expect_err("a gating slug naming no team must accept no join request either");
    let temper_services::error::ApiError::Internal(absent_msg) = absent_err else {
        panic!("expected Internal for an absent gating team, got {absent_err:?}");
    };
    assert_eq!(
        msg.replace("sd-gating-join", "<slug>"),
        absent_msg.replace("sd-gating-absent", "<slug>"),
        "a soft-deleted gating team must refuse in the same words as one that never existed"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn approval_enrolls_into_other_auto_join_teams(pool: sqlx::PgPool) {
    // Gating team = temper-system (auto_join_role watcher, seeded by migration).
    let gating_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("gating team");
    // A SECOND auto-join team that is NOT the gating team — proves the hook does
    // more than the direct gating-team insert.
    let other_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) \
         VALUES ('everyone','Everyone','member') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("other auto-join team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("invite_only");

    let joiner = common::fixtures::create_test_profile(&pool, "joiner@test.example.com").await;

    // Joiner submits a request for the gating team.
    let request_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source) \
         VALUES (gen_random_uuid(), $1, $2, 'pending', 'test') RETURNING id",
    )
    .bind(gating_id)
    .bind(joiner)
    .fetch_one(&pool)
    .await
    .expect("join request");

    // An operator approves via the service. The reviewer recorded on the decision is now the
    // authorizing admin (`admin.actor()`), so the proof IS the reviewer.
    let admin = admin_proof(&pool).await;
    access_service::review_request(
        &pool,
        &admin,
        access_service::ReviewRequestParams {
            request_id,
            decision: temper_core::types::access_gate::JoinRequestStatus::Approved,
            decision_note: None,
        },
    )
    .await
    .expect("approve");

    // The joiner is now enrolled in the OTHER auto-join team via the hook.
    let in_other: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kb_team_members WHERE team_id=$1 AND profile_id=$2)",
    )
    .bind(other_id)
    .bind(joiner)
    .fetch_one(&pool)
    .await
    .expect("check");
    assert!(
        in_other,
        "approval should enroll the profile into auto-join teams"
    );
}
