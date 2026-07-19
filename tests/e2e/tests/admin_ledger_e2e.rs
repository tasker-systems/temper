//! The admin ledger's read surface, at the production caller's level: a real server, a real
//! grant minted over HTTP, read back over HTTP.
//!
//! Task 6 of the admin-event-sink arc. Until this shipped, `admin_ledger_service` was reachable
//! from **no surface** — the ledger accumulated grant events that nothing could read, and its
//! read gate had never executed against a real caller identity. These tests are the first thing
//! to run that gate end to end.

#![cfg(feature = "test-db")]
mod common;

use serde_json::Value;
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

/// POST /api/ingest → the new resource's UUID, homed in `context_id`.
async fn ingest_into_context(app: &common::E2eTestApp, token: &str, context_id: Uuid) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": "ledger subject",
            "origin_uri": format!("test://admin-ledger-e2e/{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "slug": "ledger-subject",
            "content": "A resource whose grant should land on the admin ledger.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "owner ingests");
    let body: Value = resp.json().await.expect("ingest json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// POST /api/resources/{id}/grants — the real human grant sink, which routes through
/// `insert_grant` and so through the SQL chokepoint that writes the ledger event.
async fn grant_read(app: &common::E2eTestApp, token: &str, resource: Uuid, principal: Uuid) {
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/resources/{resource}/grants")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "principal_table": "kb_profiles",
            "principal_id": principal,
            "can_read": true,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        }))
        .send()
        .await
        .expect("grant request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "owner may grant");
}

async fn get_ledger(
    app: &common::E2eTestApp,
    token: &str,
    query: &str,
) -> (reqwest::StatusCode, Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/admin/ledger?{query}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("ledger request failed");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// The whole point of the task: an act performed over HTTP is readable over HTTP.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_grant_made_over_http_is_readable_on_the_ledger_over_http(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let _owner_id = provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app
        .client
        .contexts()
        .create("ledger-ctx", None)
        .await
        .expect("ctx");
    let resource_id = ingest_into_context(&app, &app.token, *context.id).await;
    grant_read(&app, &app.token, resource_id, stranger_id).await;

    let (status, body) = get_ledger(
        &app,
        &app.token,
        &format!("subject_kind=kb_resources&subject_id={resource_id}"),
    )
    .await;

    assert_eq!(status, reqwest::StatusCode::OK, "owner reads own subject");
    let entries = body["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 1, "the grant must be on the ledger: {body}");
    assert_eq!(entries[0]["event_type"], "grant_created");

    // The epoch is what stops an empty list from lying: "nothing since T", never "nothing ever".
    assert!(
        !body["epoch"].is_null(),
        "the response must carry the epoch: {body}"
    );
}

/// Reads deny with **404, not 403** — a 403 would confirm the ledger has something to hide
/// about this subject, which is itself the disclosure.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_administrator_gets_404_from_the_ledger(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let _owner_id = provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app
        .client
        .contexts()
        .create("ledger-ctx-2", None)
        .await
        .expect("ctx");
    let resource_id = ingest_into_context(&app, &app.token, *context.id).await;
    grant_read(&app, &app.token, resource_id, stranger_id).await;

    let query = format!("subject_kind=kb_resources&subject_id={resource_id}");

    // Prove the route EXISTS and serves this exact query before asserting anyone is denied it.
    // Without this the test is vacuous: an unmounted route also answers 404, so the assertion
    // below would pass against no implementation at all. Same request, different token — the
    // only variable left is the caller.
    let (owner_status, _) = get_ledger(&app, &app.token, &query).await;
    assert_eq!(
        owner_status,
        reqwest::StatusCode::OK,
        "precondition: the route serves this query for an authorized caller"
    );

    // The stranger holds `can_read` on the resource but NOT `can_grant`, so they cannot
    // administer grants on it and therefore may read nothing about it on the ledger.
    let (status, _) = get_ledger(&app, &stranger_token, &query).await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "deny is 404, never 403"
    );
}

/// The actor axis is self-gating (spec §11.1b): you may always read the record of acts you
/// performed. Losing a capability does not take your own history from you.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_actor_axis_lets_a_caller_read_their_own_acts(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner_id = provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app
        .client
        .contexts()
        .create("ledger-ctx-3", None)
        .await
        .expect("ctx");
    let resource_id = ingest_into_context(&app, &app.token, *context.id).await;
    grant_read(&app, &app.token, resource_id, stranger_id).await;

    let (status, body) = get_ledger(&app, &app.token, &format!("actor={owner_id}")).await;
    assert_eq!(status, reqwest::StatusCode::OK, "own history is readable");
    let entries = body["entries"].as_array().expect("entries array");
    assert!(
        entries.iter().any(|e| e["event_type"] == "grant_created"),
        "the caller's own grant must appear on their actor axis: {body}"
    );

    // Reading SOMEONE ELSE's history is an audit, and audits are admin-only.
    let (status, _) = get_ledger(&app, &stranger_token, &format!("actor={owner_id}")).await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "a non-admin may not audit another actor"
    );
}

/// Two axes that gate differently and answer different questions. Picking one for the caller
/// when they named both would answer a question they did not ask, under a gate they did not
/// expect — so ambiguity is refused rather than resolved.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_axes_are_exclusive_and_neither_defaults(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner_id = provision(&app, &app.token).await;

    for (query, why) in [
        (
            format!("actor={owner_id}&subject_kind=kb_resources&subject_id={owner_id}"),
            "both axes at once",
        ),
        (String::new(), "no axis at all"),
        (
            "subject_kind=kb_resources".to_string(),
            "half a subject is not a subject",
        ),
        (
            format!("subject_kind=kb_nonsense&subject_id={owner_id}"),
            "unknown subject_kind",
        ),
    ] {
        let (status, _) = get_ledger(&app, &app.token, &query).await;
        assert_eq!(
            status,
            reqwest::StatusCode::BAD_REQUEST,
            "{why} must be a 400"
        );
    }
}
