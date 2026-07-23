#![cfg(feature = "test-db")]
//! Context-ownership-transfer e2e (Seq 20): drives the real Axum server + Postgres to prove
//! `temper context transfer` / `POST /api/contexts/{id}/reassign` is a genuine ownership
//! change that grants a team *authorship* — the single path to shared writing (read-sharing
//! stays `share_context`). Transferring a personal context `C` to a team `T` makes `C`
//! team-owned, so `T`'s authoring-role members can both read and **write** into it, while a
//! non-administrator of `C` cannot transfer it.
//!
//! Modeled on `context_share_e2e.rs` (same admin-minting root step + `POST /api/ingest`
//! anchoring idiom) — but where the share test proves read-reach via `GET`, this proves the
//! new *authorship* capability by having a team member author a fresh resource into the
//! now-team-owned context (`POST /api/ingest` → 200), which is impossible while the context
//! is personal.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::context::ReassignContextRequest;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
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

/// The irreducible operator root step: configure gating + mint first admin.
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
    // D11: admin-ness is `approved` standing + a `kb_principal_governance` grant. Neither the
    // Phase-2-retired `system_access` column nor gating ownership confers it.
    common::approved_admin(pool, admin_id).await;
}

/// `POST /api/ingest` homed in `context_id` (a bare UUID `context_ref`) as `token`; returns
/// the wire status so the caller can assert both allow (200) and deny.
async fn ingest_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    slug: &str,
) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": format!("ctx-transfer {slug}"),
            "origin_uri": format!("test://context-transfer-e2e/{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource authored into a context.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    let status = resp.status();
    let body = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// `GET /api/resources/{id}` status — the full-stack visibility oracle: 200 visible, 404 deny.
async fn show_status(app: &common::E2eTestApp, token: &str, resource: Uuid) -> StatusCode {
    app.reqwest_client
        .get(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show request failed")
        .status()
}

/// Wire-level status of `POST /api/contexts/{context}/reassign` as `token`.
async fn transfer_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    team_id: Uuid,
) -> StatusCode {
    app.reqwest_client
        .post(app.url(&format!("/api/contexts/{context_id}/reassign")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "to_team_id": team_id }))
        .send()
        .await
        .expect("transfer request")
        .status()
}

async fn context_owner(pool: &sqlx::PgPool, context_id: Uuid) -> (String, Uuid) {
    let row: (String, Uuid) =
        sqlx::query_as("SELECT owner_table, owner_id FROM kb_contexts WHERE id = $1")
            .bind(context_id)
            .fetch_one(pool)
            .await
            .expect("owner query");
    row
}

/// The authorship oracle: `can_modify_resource(profile, resource)` — the container-write
/// cascade that team ownership widens. (The full-stack read is proven via `GET` above; this
/// asserts the *write* capability the transfer confers.)
async fn can_modify(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("can_modify_resource query")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn transfer_makes_context_team_owned_and_members_can_author(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // Admin owns a personal context and homes a resource in it.
    let context = app
        .client
        .contexts()
        .create("proj", None)
        .await
        .expect("admin creates context");
    let (ing, body) = ingest_status(&app, &app.token, *context.id, "admin-doc").await;
    assert_eq!(ing, StatusCode::OK, "admin authors into their own context");
    let resource_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // Admin creates a team and adds the member at an authoring role.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "acme".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("admin creates team");
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
        .expect("admin adds member");

    // Pre-transfer: the context is personal, so the member can neither see nor author in it.
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        StatusCode::NOT_FOUND,
        "member cannot see a resource in the admin's personal context"
    );
    assert!(
        !can_modify(&pool, member_id, resource_id).await,
        "member cannot author in the admin's personal context before transfer"
    );

    // ── The transfer, via the client (mirrors `temper context transfer`). ──
    let outcome = app
        .client
        .contexts()
        .reassign(
            *context.id,
            &ReassignContextRequest {
                to_team_id: team.id,
            },
        )
        .await
        .expect("admin transfers context to team");
    assert!(outcome.reassigned, "first transfer flips ownership");
    assert_eq!(
        outcome.owner_ref, "+acme",
        "owner ref is the decorated team"
    );
    assert!(
        outcome.inherited_shares.is_empty(),
        "a fresh context has no prior shares to inherit"
    );
    assert!(
        outcome.inherited_read_grants.is_empty(),
        "a fresh context has no prior read-grants to inherit"
    );
    assert_eq!(
        context_owner(&pool, *context.id).await,
        ("kb_teams".to_string(), team.id),
        "the context is now team-owned"
    );

    // Post-transfer: the member can read the admin's resource (full-stack GET) AND now has
    // authorship over it via the container-write cascade the team ownership widened.
    assert_eq!(
        show_status(&app, &member_token, resource_id).await,
        StatusCode::OK,
        "member reads the resource once the context is team-owned"
    );
    assert!(
        can_modify(&pool, member_id, resource_id).await,
        "member gains authorship into the team-owned context (shared authorship)"
    );

    // Idempotent: a second transfer to the same team is a no-op.
    let again = app
        .client
        .contexts()
        .reassign(
            *context.id,
            &ReassignContextRequest {
                to_team_id: team.id,
            },
        )
        .await
        .expect("idempotent re-transfer");
    assert!(!again.reassigned, "already team-owned → no-op");
}

/// A transfer SURFACES (does not sweep) the read-reach it inherits (spec D3): a prior
/// `kb_team_contexts` share and an explicit `kb_access_grants` context read-grant both come
/// back in the outcome so the new owner can prune deliberately.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn transfer_surfaces_inherited_shares_and_read_grants(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // Admin owns a personal context and the target team T.
    let context = app
        .client
        .contexts()
        .create("proj", None)
        .await
        .expect("admin creates context");
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "acme".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("admin creates target team");

    // Seed residual reach BEFORE the transfer:
    //   1) a share to a second team `other`  → kb_team_contexts
    //   2) a context read-grant to a `viewer` profile → kb_access_grants(subject='kb_contexts')
    let other_team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('other','Other') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("other team");
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(*context.id)
        .bind(other_team)
        .execute(&pool)
        .await
        .expect("seed share");
    let viewer: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('viewer','viewer') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("viewer profile");
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, false, $3)",
    )
    .bind(*context.id)
    .bind(viewer)
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("seed read-grant");

    // Transfer C → T; the outcome reports the inherited reach.
    let outcome = app
        .client
        .contexts()
        .reassign(
            *context.id,
            &ReassignContextRequest {
                to_team_id: team.id,
            },
        )
        .await
        .expect("admin transfers context to team");

    assert!(outcome.reassigned, "the transfer happens");
    assert_eq!(
        outcome.inherited_shares.len(),
        1,
        "the share to `other` is surfaced"
    );
    assert_eq!(outcome.inherited_shares[0].team_id, other_team);
    assert_eq!(outcome.inherited_shares[0].team_ref, "+other");
    assert_eq!(
        outcome.inherited_read_grants.len(),
        1,
        "the viewer read-grant is surfaced"
    );
    assert_eq!(outcome.inherited_read_grants[0].principal_id, viewer);
    assert_eq!(outcome.inherited_read_grants[0].principal_ref, "@viewer");

    // Surfaced, NOT swept: the share and grant still exist after the transfer.
    let shares_remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_team_contexts WHERE context_id = $1")
            .bind(*context.id)
            .fetch_one(&pool)
            .await
            .expect("count shares");
    assert_eq!(shares_remaining, 1, "transfer does not sweep the share");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_context_administrator_cannot_transfer(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // The ADMIN owns this context; the member merely owns a team.
    let context = app
        .client
        .contexts()
        .create("admins-ctx", None)
        .await
        .expect("admin creates context");
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "member-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("admin creates team");
    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest {
                profile_id: member_id,
                role: TeamRole::Maintainer,
            },
        )
        .await
        .expect("admin adds member as maintainer");

    // The member manages the team but does NOT administer the admin's context → 403.
    assert_eq!(
        transfer_status(&app, &member_token, *context.id, team.id).await,
        StatusCode::FORBIDDEN,
        "managing the target team is not enough — the caller must administer the context"
    );
    assert_eq!(
        context_owner(&pool, *context.id).await,
        ("kb_profiles".to_string(), admin_id),
        "a denied transfer changes nothing"
    );
}
