//! The route table's human-only tiers, through the real app.
//!
//! `/api/profile` is split across two groups behind two tiers: GET in `auth_only_status_routes`
//! (machine-admitting `AuthOnly`), PATCH in `auth_only_routes` (`HumanAuthOnly`). Merging two
//! routers that share a path is axum's `merge_for_path`, which refuses only a repeated method; these
//! pin that both methods still route, each behind its own tier. The admin group's `HumanGated` tier
//! refuses a machine before `require_system_access`, so an unapproved machine is told it is a
//! machine rather than that it lacks system access.
#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;
use temper_services::auth::MACHINE_PRINCIPAL_REFUSAL;
use uuid::Uuid;

const MACHINE_CLIENT: &str = "human-tier-routing-agent";

/// A registered machine: a profile with a `kb_machine_clients` row, and no standing (born
/// `Denied`) unless the caller approves it.
async fn seed_machine(pool: &PgPool) -> Uuid {
    let profile = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'human-tier-routing-agent', 'human-tier-routing-agent', NULL, '{}')",
    )
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed machine profile");
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'test', $2, $2)",
    )
    .bind(MACHINE_CLIENT)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed machine registration");
    profile
}

/// Assert `resp` is the machine refusal: 403, `FORBIDDEN_DETAIL`, the fixed sentence.
async fn assert_machine_refusal(resp: reqwest::Response, door: &str) {
    assert_eq!(resp.status().as_u16(), 403, "{door}: a machine is refused");
    let body: Value = resp.json().await.expect("error body");
    assert_eq!(
        body["error"]["code"],
        temper_core::error::FORBIDDEN_DETAIL_CODE,
        "{door}: {body}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains(MACHINE_PRINCIPAL_REFUSAL)),
        "{door}: {body}"
    );
}

/// A person reaches both halves of `/api/profile`: the split did not drop either method.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_reaches_both_profile_methods(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let token = common::generate_test_jwt(
        &format!("route-split-{}", Uuid::new_v4()),
        &format!("route-split-{}@example.com", Uuid::new_v4()),
    );

    let get = app
        .client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("GET");
    assert_eq!(get.status().as_u16(), 200, "GET /api/profile routes");

    let patch = app
        .client
        .patch(app.url("/api/profile"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "display_name": "renamed" }))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(patch.status().as_u16(), 200, "PATCH /api/profile routes");
    let body: Value = patch.json().await.expect("profile");
    assert_eq!(body["display_name"], "renamed");
}

/// A machine keeps the profile read and is refused the profile write — at the tier, since the two
/// methods of one path now sit behind different stacks.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_reads_its_profile_and_is_refused_the_write(pool: PgPool) {
    let machine = seed_machine(&pool).await;
    let app = common::setup_test_app(pool).await;
    let token = common::generate_machine_jwt(MACHINE_CLIENT);

    let get = app
        .client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("GET");
    assert_eq!(get.status().as_u16(), 200, "a machine reads its profile");
    let body: Value = get.json().await.expect("profile");
    assert_eq!(body["id"], machine.to_string());

    let patch = app
        .client
        .patch(app.url("/api/profile"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "display_name": "renamed" }))
        .send()
        .await
        .expect("PATCH");
    assert_machine_refusal(patch, "PATCH /api/profile").await;
}

/// An unapproved machine on an admin door meets the machine refusal, not `SYSTEM_ACCESS_REQUIRED`:
/// `refuse_machine` is outer to `require_system_access` in the `HumanGated` stack. On the
/// machine-admitting gated tier the same machine meets the system-access refusal instead.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unapproved_machine_is_refused_as_a_machine_on_an_admin_door(pool: PgPool) {
    seed_machine(&pool).await;
    let app = common::setup_test_app(pool).await;
    let token = common::generate_machine_jwt(MACHINE_CLIENT);

    let admin = app
        .client
        .get(app.url("/api/access/admin/settings"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("admin door");
    assert_machine_refusal(admin, "GET /api/access/admin/settings").await;

    let gated = app
        .client
        .get(app.url("/api/contexts"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("gated door");
    assert_eq!(gated.status().as_u16(), 403);
    let body: Value = gated.json().await.expect("error body");
    assert_eq!(body["error"]["code"], "SYSTEM_ACCESS_REQUIRED", "{body}");
}
