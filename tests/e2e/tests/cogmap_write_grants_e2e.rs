#![cfg(feature = "test-db")]
//! Cogmap-write tightening (Q-A, D3b) end-to-end: drives the REAL Axum server, real Postgres, real
//! JWT auth, and the REAL grant/revoke surface via `temper-client`. Authorship into a cognitive map
//! is now an explicit `can_write` grant, NOT team membership.
//!
//! Scenarios (the cross-stack ones e2e uniquely validates):
//!   (a) A non-member gains cogmap-write ONLY via an explicit grant minted through the production
//!       grant caller; revoke removes it again.
//!   (b) The creator authors their freshly-created, UNBOUND map (the creator-seed grant).
//!   (c) A non-admin, non-granted user cannot author the operator-governed L0 kernel (auto-join
//!       exclusion — they'd have passed under the old flat membership stub).

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::cognitive_maps::{CogmapGrantBody, CogmapRevokeBody};
use temper_core::types::reconcile::CreateCogmapRequest;

/// The reserved L0 kernel cognitive map (`20260625000001_l0_kernel_cogmap.sql`).
const L0_KERNEL: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// A genesis request with fixed ids (empty charter — no ONNX, runs on plain `cargo make test-e2e`).
fn genesis_request(cogmap: Uuid, telos: Uuid) -> CreateCogmapRequest {
    CreateCogmapRequest {
        cogmap_id: Some(cogmap),
        telos_resource_id: Some(telos),
        name: "Grantable map".to_string(),
        telos_title: "Grant telos".to_string(),
        telos: None,
    }
}

/// Pre-flight a token by hitting GET /api/profile (auto-provisions the profile), returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse")
}

/// Enrol a profile as a `watcher` of temper-system, so it passes the invite_only system-access
/// middleware (system access) WITHOUT being an admin or a member of any cogmap's team — the
/// production shape for a "reaches the handler but is gated by the authorable predicate" user.
async fn add_system_watcher(pool: &sqlx::PgPool, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug = 'temper-system' \
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(profile)
    .execute(pool)
    .await
    .expect("add system watcher");
}

/// POST /api/ingest homed in a cognitive map, as `token`. Returns the raw response.
async fn post_cogmap_ingest(
    app: &common::E2eTestApp,
    token: &str,
    cogmap: Uuid,
    title: &str,
    slug: &str,
) -> reqwest::Response {
    app.reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "origin_uri": format!("test://cogmap-write/{}", Uuid::new_v4()),
            "context_ref": "",
            "home_cogmap_id": cogmap.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource authored into a cognitive map.",
        }))
        .send()
        .await
        .expect("ingest request failed")
}

// ── (a) non-member gains cogmap-write ONLY via an explicit grant (and revoke removes it) ──────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_member_authors_only_via_explicit_grant(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let cogmap = Uuid::from_u128(0x019f0ccc_1111_7000_8000_000000000001);
    let telos = Uuid::from_u128(0x019f0ccc_1111_7000_8000_000000000002);
    app.client
        .cognitive_maps()
        .create_cognitive_map(&genesis_request(cogmap, telos))
        .await
        .expect("admin genesis");

    // A second user: system access (watcher of temper-system) but NOT admin, NOT granted — reaches
    // the authorable gate, which is what we exercise.
    let second_token = common::generate_second_user_jwt();
    let second_id = provision_profile(&app, &second_token).await;
    add_system_watcher(&pool, second_id).await;

    // Before any grant: authoring is denied (403).
    let denied =
        post_cogmap_ingest(&app, &second_token, cogmap, "before grant", "before-grant").await;
    assert_eq!(
        denied.status(),
        StatusCode::FORBIDDEN,
        "a non-member with no grant cannot author the map"
    );

    // The admin grants the second user can_write through the PRODUCTION grant caller.
    let out = app
        .client
        .cognitive_maps()
        .grant(
            cogmap,
            &CogmapGrantBody {
                principal_table: "kb_profiles".to_string(),
                principal_id: second_id,
                can_read: true,
                can_write: true,
                can_delete: false,
                can_grant: false,
            },
        )
        .await
        .expect("admin grants write");
    assert!(out.granted, "a fresh grant reports granted=true");

    // Now the second user authors successfully.
    let ok = post_cogmap_ingest(&app, &second_token, cogmap, "after grant", "after-grant").await;
    assert_eq!(
        ok.status(),
        StatusCode::OK,
        "an explicit can_write grant lets the non-member author"
    );

    // Revoke removes authoring again.
    let rev = app
        .client
        .cognitive_maps()
        .revoke(
            cogmap,
            &CogmapRevokeBody {
                principal_table: "kb_profiles".to_string(),
                principal_id: second_id,
            },
        )
        .await
        .expect("admin revokes");
    assert!(rev.revoked, "revoke deletes the grant");

    let denied_again =
        post_cogmap_ingest(&app, &second_token, cogmap, "after revoke", "after-revoke").await;
    assert_eq!(
        denied_again.status(),
        StatusCode::FORBIDDEN,
        "after revoke, authoring is denied again"
    );
}

// ── (b) the creator authors their freshly-created, unbound map (creator-seed) ─────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn creator_authors_their_unbound_map(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let cogmap = Uuid::from_u128(0x019f0ccc_2222_7000_8000_000000000001);
    let telos = Uuid::from_u128(0x019f0ccc_2222_7000_8000_000000000002);
    app.client
        .cognitive_maps()
        .create_cognitive_map(&genesis_request(cogmap, telos))
        .await
        .expect("admin genesis");

    // No bind — the map is unbound. The creator-seed grant (write) lets the creator author anyway.
    let resp = post_cogmap_ingest(&app, &app.token, cogmap, "creator note", "creator-note").await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "the creator authors their own unbound map via the creator-seed grant"
    );
}

// ── (c) a non-admin, non-granted user cannot author the L0 kernel (auto-join exclusion) ───────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn arbitrary_user_cannot_author_l0(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    // A second user with system access (reaches the authorable gate) but no grant on L0. Under the
    // OLD flat stub they'd have authored L0 (auto-join member of temper-system, which L0 is joined
    // to); post-Q-A + backfill-exclusion they cannot.
    let second_token = common::generate_second_user_jwt();
    let second_id = provision_profile(&app, &second_token).await;
    add_system_watcher(&pool, second_id).await;

    let resp =
        post_cogmap_ingest(&app, &second_token, L0_KERNEL, "kernel edit", "kernel-edit").await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "an arbitrary user cannot author the operator-governed L0 kernel"
    );
}
