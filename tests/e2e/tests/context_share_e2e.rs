#![cfg(feature = "test-db")]
//! Context-share e2e (plan Task A5): drives the real Axum server + Postgres to prove the
//! `temper context share`/`unshare` surface (Tasks A1-A4, already committed) is a genuine
//! READ-reach grant — sharing a context `C` into a team `T` widens `resources_visible_to`
//! for `T`'s members, and unsharing reverses it. Also proves the admin gate: a non-admin's
//! share attempt is `403`.
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
    body["id"].as_str().expect("id").parse().expect("uuid")
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
