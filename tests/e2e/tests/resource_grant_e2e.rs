#![cfg(feature = "test-db")]
mod common;

use serde_json::Value;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use uuid::Uuid;

/// GET /api/profile → this token's profile UUID (mints the profile on first hit).
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    let body: Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// POST /api/ingest → the new resource's UUID. Homes the resource in `context_id`.
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
            "origin_uri": format!("test://resource-grant-e2e/{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource owned by the granter, shared to a team by capability grant.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "owner ingests into own context"
    );
    let body: Value = resp.json().await.expect("ingest json");
    body["id"].as_str().expect("resource id").parse().expect("uuid")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_can_administer_grant_seam(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner_id = provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app
        .client
        .contexts()
        .create("seam-ctx", None)
        .await
        .expect("ctx");
    let resource_id =
        ingest_into_context(&app, &app.token, *context.id, "seam doc", "seam-doc").await;

    // The owner-grant seam: the resource's owner CAN administer grants; a stranger cannot.
    let owner_can: Option<bool> =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)")
            .bind(owner_id)
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .expect("can() query");
    assert_eq!(
        owner_can,
        Some(true),
        "resource owner may administer grants (the new seam)"
    );

    let stranger_can: Option<bool> =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)")
            .bind(stranger_id)
            .bind(resource_id)
            .fetch_one(&pool)
            .await
            .expect("can() query");
    assert_eq!(
        stranger_can,
        Some(false),
        "a non-owner, non-admin cannot administer grants"
    );
}
