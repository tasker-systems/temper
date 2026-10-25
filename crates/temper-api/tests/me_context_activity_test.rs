#![cfg(feature = "test-db")]
//! `GET /api/me/context-activity` through the real app: refused without a token, and for a signed-in
//! reader exactly one `{context_id, last_changed_at}` row per context they changed something in.
//! What the rows mean — whose changes, which events, which contexts — is witnessed against the
//! service in `temper-services/tests/changed_by_filter_test.rs`; this holds the route to it.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

mod common;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_route_answers_the_callers_own_activity_and_refuses_the_anonymous(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let anonymous = app
        .client
        .get(app.url("/api/me/context-activity"))
        .send()
        .await
        .expect("request");
    assert_eq!(anonymous.status().as_u16(), 401);

    let sub = format!("test-sub-{}", Uuid::new_v4());
    let email = format!("activity-{}@example.com", Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);
    let provisioned = app
        .client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("provision");
    assert_eq!(provisioned.status().as_u16(), 200);
    common::fixtures::approve_standing_by_email(&app.pool, &email).await;

    let get = || async {
        let resp = app
            .client
            .get(app.url("/api/me/context-activity"))
            .bearer_auth(&token)
            .send()
            .await
            .expect("activity");
        assert_eq!(resp.status().as_u16(), 200);
        resp.json::<Vec<Value>>().await.expect("a JSON array")
    };
    assert!(get().await.is_empty(), "a new reader has changed nothing");

    // One authored change in the reader's default context, under one of their own emitters.
    let context: Uuid = sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c JOIN kb_profiles p ON p.id = c.owner_id \
          WHERE c.owner_table = 'kb_profiles' AND p.email = $1 AND c.slug = 'default'",
    )
    .bind(&email)
    .fetch_one(&app.pool)
    .await
    .expect("the default context");
    sqlx::query(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id) \
         SELECT et.id, en.id, 'kb_contexts', $2 \
           FROM kb_event_types et, kb_entities en JOIN kb_profiles p ON p.id = en.profile_id \
          WHERE et.name = 'context_renamed' AND p.email = $1 \
          ORDER BY en.id LIMIT 1",
    )
    .bind(&email)
    .bind(context)
    .execute(&app.pool)
    .await
    .expect("emit");

    let rows = get().await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = rows[0].as_object().expect("an object");
    let mut keys: Vec<&str> = row.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec!["context_id", "last_changed_at"],
        "nothing but the two fields"
    );
    assert_eq!(row["context_id"], context.to_string());
}
