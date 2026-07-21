#![cfg(feature = "test-db")]
//! Context-share e2e (plan Task A5): drives the real Axum server + Postgres to prove the
//! `temper context share`/`unshare` surface (Tasks A1-A4, already committed) is a genuine
//! READ-reach grant — sharing a context `C` into a team `T` widens `resources_visible_to`
//! for `T`'s members, and unsharing reverses it. Also proves the `can_share` authorization
//! matrix (issue #367 relaxation, mirroring `bind_cogmap_e2e.rs`'s `can_bind` matrix): a
//! bare non-admin is denied, but a caller who administers the context AND manages the target
//! team may share — while sharing into the gating/root team stays admin-only.
//!
//! Modeled on `admin_surface_e2e.rs` (the admin-minting root step: promote via
//! `kb_profiles.system_access='admin'`, which the auto-join trigger then upgrades to `owner`
//! of `temper-system`) and `cogmap_read_up_flip_e2e.rs` (drive `GET /api/resources/{id}` as
//! the full-stack visibility oracle, not an isolated-predicate test — the lesson from #219
//! is that isolated-DB predicate tests are not enough; the deny code + auth + handler must
//! agree). The resource is authored through the production `POST /api/ingest` path, homed
//! directly in the context via a bare context UUID in `context_ref` (accepted verbatim —
//! see `IngestPayload::context_ref` doc comment and `resources.rs::create`'s
//! `ContextRef::Id(...)` resolution, which is the same underlying `resolve_context_ref`).

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::context::ShareContextRequest;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
/// Copied verbatim from `admin_surface_e2e.rs` / `cogmap_read_up_flip_e2e.rs`.
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, __pid).await;
    __pid
}

/// The irreducible 2-UPDATE operator root step: configure gating + mint first admin.
/// Copied verbatim from `admin_surface_e2e.rs::root_bootstrap_first_admin`.
async fn root_bootstrap_first_admin(pool: &sqlx::PgPool, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .expect("gating");
    sqlx::query("UPDATE kb_profiles SET system_access='admin' WHERE id=$1")
        .bind(admin_id)
        .execute(pool)
        .await
        .expect("promote first admin"); // trigger mints owner of temper-system
                                        // D11: is_system_admin reads governance, has_system_access reads standing; the column + gating
                                        // ownership above confer neither. Grant both so the bootstrapped admin can actually act.
    common::approved_admin(pool, admin_id).await;
}

/// `POST /api/ingest` homed in `context_id` (a bare UUID `context_ref`), as `token`.
/// Returns the created resource id. Mirrors `ingest_into_cogmap` in
/// `cogmap_read_up_flip_e2e.rs`, substituting a context anchor for a cogmap anchor.
async fn ingest_into_context(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    title: &str,
    slug: &str,
) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "origin_uri": format!("test://context-share-e2e/{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource homed in the admin's context, later shared into a team.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "admin ingests into their own context"
    );
    let body: Value = resp.json().await.expect("ingest json parse");
    body["id"]
        .as_str()
        .expect("ingested resource id missing")
        .parse()
        .expect("resource id parse")
}

/// `GET /api/resources/{id}` status — the full-stack visibility oracle (same idiom as
/// `cogmap_read_up_flip_e2e.rs::show_status`): 200 = visible, 404 = deny-as-absence.
async fn show_status(app: &common::E2eTestApp, token: &str, resource: Uuid) -> StatusCode {
    app.reqwest_client
        .get(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show request failed")
        .status()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_share_widens_visibility_admin_gated_and_reversible(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Admin (drives the app's built-in `app.client`/`app.token`) and a second,
    // non-admin profile that will become team `T`'s member.
    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;

    root_bootstrap_first_admin(&pool, admin_id).await;

    // Admin creates a profile-owned context `C` (owner = None ⇒ the caller's own
    // profile, per `ContextClient::create`'s doc comment) and homes a resource in it.
    let context = app
        .client
        .contexts()
        .create("shared-ctx", None)
        .await
        .expect("admin creates context");
    let resource_id = ingest_into_context(
        &app,
        &app.token,
        *context.id,
        "context-share doc",
        "context-share-doc",
    )
    .await;

    // Admin creates team `T` (the caller becomes its `owner`, per `TeamsClient::create`'s
    // doc comment). The member is NOT yet a member of `T`.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "context-share-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("admin creates team");

    // ── Admin gate: a non-admin's share attempt is 403. Hit the raw endpoint with the
    // member's token directly (rather than building a second full `TemperClient`) — the
    // member has no admin session, and the assertion only needs the wire-level status.
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/contexts/{}/teams", *context.id)))
        .header("Authorization", format!("Bearer {member_token}"))
        .json(&serde_json::json!({ "team_id": team.id }))
        .send()
        .await
        .expect("non-admin share attempt");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "non-admin cannot share a context into a team"
    );

    // Admin (owner of `T`) adds the member to `T`.
    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest {
                profile_id: member_id,
                role: TeamRole::Member,
            },
        )
        .await
        .expect("admin adds member to team");

    // Pre-share: the member is in `T`, but `C` is not yet shared into `T` — no visibility.
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        StatusCode::NOT_FOUND,
        "member cannot see the context's resource before the share (deny-as-absence)"
    );

    // Admin shares `C` into `T` via the client (mirrors `temper context share`'s
    // `ContextClient::share_team` call in `context_cmd.rs::share_remote`).
    let outcome = app
        .client
        .contexts()
        .share_team(*context.id, &ShareContextRequest { team_id: team.id })
        .await
        .expect("admin shares context into team");
    assert!(
        outcome.shared,
        "first share call inserts the kb_team_contexts row"
    );
    assert_eq!(outcome.context_id, *context.id);
    assert_eq!(outcome.team_id, team.id);

    // Idempotent no-op second call.
    let outcome2 = app
        .client
        .contexts()
        .share_team(*context.id, &ShareContextRequest { team_id: team.id })
        .await
        .expect("admin re-shares (idempotent)");
    assert!(
        !outcome2.shared,
        "second share call is a no-op (row already existed)"
    );

    // ── Post-share: T's member now sees the resource via the widened read-reach.
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        StatusCode::OK,
        "team member sees the context's resource once the context is shared into their team"
    );

    // Admin unshares — reverses the grant (mirrors `temper context unshare`'s
    // `ContextClient::unshare_team` call in `context_cmd.rs::unshare_remote`).
    let unshared = app
        .client
        .contexts()
        .unshare_team(*context.id, team.id)
        .await
        .expect("admin unshares");
    assert!(
        unshared.unshared,
        "unshare call deletes the kb_team_contexts row"
    );

    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        StatusCode::NOT_FOUND,
        "member loses visibility once the context is unshared from their team"
    );

    // Second unshare is a no-op-safe call (no row to delete).
    let unshared2 = app
        .client
        .contexts()
        .unshare_team(*context.id, team.id)
        .await
        .expect("admin unshares again (no-op)");
    assert!(!unshared2.unshared, "second unshare call is a no-op");
}

// ── can_share relaxation matrix (issue #367) — mirrors bind_cogmap_e2e.rs ──────────────────

/// Create a profile-owned context named `name` as `token` (owner defaults to the caller),
/// returning its id. Drives the raw `POST /api/contexts` endpoint so an arbitrary token can
/// own the context (the built-in `app.client` is always the admin).
async fn create_context_as(app: &common::E2eTestApp, token: &str, name: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .expect("create context request");
    assert_eq!(resp.status(), StatusCode::CREATED, "context created");
    let body: Value = resp.json().await.expect("context json");
    body["id"]
        .as_str()
        .expect("context id")
        .parse()
        .expect("uuid")
}

/// Make `profile` a member of a fresh NON-gating team at `role`; returns the team id.
async fn team_with_role(pool: &sqlx::PgPool, profile: Uuid, role: &str) -> Uuid {
    let team_id = Uuid::now_v7();
    let slug = format!(
        "share-role-team-{}",
        &Uuid::new_v4().simple().to_string()[..8]
    );
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(team_id)
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert team");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(profile)
    .bind(role)
    .execute(pool)
    .await
    .expect("insert membership");
    team_id
}

/// Wire-level status of `POST /api/contexts/{context}/teams` as `token`.
async fn share_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    team_id: Uuid,
) -> StatusCode {
    app.reqwest_client
        .post(app.url(&format!("/api/contexts/{context_id}/teams")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "team_id": team_id }))
        .send()
        .await
        .expect("share request")
        .status()
}

/// `true` when a `kb_team_contexts(context_id, team_id)` row exists.
async fn share_exists(pool: &sqlx::PgPool, context_id: Uuid, team_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM kb_team_contexts WHERE context_id = $1 AND team_id = $2)",
    )
    .bind(context_id)
    .bind(team_id)
    .fetch_one(pool)
    .await
    .expect("kb_team_contexts exists query")
}

// ── (c) a team maintainer who owns the context may share it into their team (+ symmetric unshare) ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_owner_and_team_maintainer_may_share(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // The second user owns the context AND maintains the target team — the two-sided gate.
    let context_id = create_context_as(&app, &member_token, "owned-ctx").await;
    let team_id = team_with_role(&pool, member_id, "maintainer").await;

    assert_eq!(
        share_status(&app, &member_token, context_id, team_id).await,
        StatusCode::OK,
        "a context owner who maintains the target team may share (no instance-admin required)"
    );
    assert!(
        share_exists(&pool, context_id, team_id).await,
        "the share row was written"
    );

    // Symmetric unshare: the same principal may reverse it.
    let unshare = app
        .reqwest_client
        .delete(app.url(&format!("/api/contexts/{context_id}/teams/{team_id}")))
        .header("Authorization", format!("Bearer {member_token}"))
        .send()
        .await
        .expect("unshare request");
    assert_eq!(
        unshare.status(),
        StatusCode::OK,
        "same principal may unshare"
    );
    assert!(
        !share_exists(&pool, context_id, team_id).await,
        "the share row was removed"
    );
}

// ── (d) a team maintainer who does NOT administer the context is denied (context-side gate) ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_maintainer_without_context_admin_denied(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // The ADMIN owns this context; the second user only maintains the team.
    let admins_context = app
        .client
        .contexts()
        .create("admins-ctx", None)
        .await
        .expect("admin creates context");
    let team_id = team_with_role(&pool, member_id, "maintainer").await;

    assert_eq!(
        share_status(&app, &member_token, *admins_context.id, team_id).await,
        StatusCode::FORBIDDEN,
        "maintaining the team is not enough — the caller must also administer the context"
    );
    assert!(
        !share_exists(&pool, *admins_context.id, team_id).await,
        "a denied share writes nothing"
    );
}

// ── (e) a mere team member (owns the context but cannot manage the team) is denied ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_owner_who_is_mere_team_member_denied(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let context_id = create_context_as(&app, &member_token, "owned-ctx-2").await;
    let team_id = team_with_role(&pool, member_id, "member").await; // not can_manage

    assert_eq!(
        share_status(&app, &member_token, context_id, team_id).await,
        StatusCode::FORBIDDEN,
        "owning the context is not enough — the caller must manage (owner/maintainer) the team"
    );
    assert!(
        !share_exists(&pool, context_id, team_id).await,
        "a denied share writes nothing"
    );
}

// ── (f) sharing into the gating/root team stays admin-only (escalation guard) ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn share_into_gating_team_denied_for_non_admin(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // The second user owns the context and is even a MAINTAINER of the gating team — but sharing
    // into the root team is an instance-level escalation kept admin-only.
    let context_id = create_context_as(&app, &member_token, "owned-ctx-3").await;
    let gating_team_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
            .fetch_one(&pool)
            .await
            .expect("gating team id");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'maintainer') \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'maintainer'",
    )
    .bind(gating_team_id)
    .bind(member_id)
    .execute(&pool)
    .await
    .expect("promote to maintainer of gating team");

    assert_eq!(
        share_status(&app, &member_token, context_id, gating_team_id).await,
        StatusCode::FORBIDDEN,
        "sharing into the gating team must stay admin-only even for a maintainer"
    );
    assert!(
        !share_exists(&pool, context_id, gating_team_id).await,
        "no escalation share written"
    );
}
