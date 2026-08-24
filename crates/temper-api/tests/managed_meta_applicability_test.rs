//! No door stores a state its kind does not carry.
//!
//! The doc-type definitions say which states a kind of work carries but did not exclude the
//! states of other kinds — every schema is `additionalProperties: true`, and the typed managed
//! tier carries `stage` as a plain optional string on every kind. So a **task** stage set on a
//! **goal** deserialized, validated clean, and stored. These ask a running service, because the
//! gate is shared by every door and only a wire test establishes that the door in front of it
//! actually reaches it.
//!
//! **The rule is a property of the act, not of the data:** a door may not *introduce* a state
//! its kind does not carry; a value already stored may be restated. A production measurement
//! taken before this change found exactly two live resources holding a stray field — a
//! `concept` with `temper-status` and a `session` with `temper-branch` — and no door can remove
//! a managed property (`properties_from_meta` skips nulls rather than retracting the row). An
//! unconditional refusal would wedge them with no exit.
#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use sqlx::PgPool;

/// Create one resource of `doc_type` through the ingest door and return its id.
async fn create(
    app: &common::TestApp,
    token: &str,
    context_id: &uuid::Uuid,
    doc_type: &str,
    managed_meta: serde_json::Value,
) -> String {
    let marker = uuid::Uuid::now_v7();
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": format!("applicability {marker}"),
            "origin_uri": format!("test://applicability-{marker}"),
            "context_ref": context_id.to_string(),
            "doc_type_name": doc_type,
            "slug": format!("applicability-{marker}"),
            "content": "body",
            "managed_meta": managed_meta,
            "open_meta": {}
        }))
        .send()
        .await
        .expect("ingest request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("ingest response JSON");
    assert_eq!(status, 200, "setup create must succeed; body: {body}");
    body["id"]
        .as_str()
        .expect("id in ingest response")
        .to_owned()
}

/// The write the clause names. Before this gate it returned 200 and stored the stray.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_create_may_not_store_a_state_the_kind_does_not_carry(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("applic-create-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let marker = uuid::Uuid::now_v7();
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": "a goal wearing a task's stage",
            "origin_uri": format!("test://applic-{marker}"),
            "context_ref": context_id.to_string(),
            "doc_type_name": "goal",
            "slug": format!("applic-{marker}"),
            "content": "body",
            "managed_meta": { "temper-stage": "backlog" },
            "open_meta": {}
        }))
        .send()
        .await
        .expect("ingest request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "a goal does not carry a task's stage, so the door must refuse it"
    );
    let text = resp.text().await.expect("error body");
    assert!(
        text.contains("temper-stage") && text.contains("goal"),
        "the refusal must name the field and the kind so the caller can act on it: {text}"
    );
}

/// The same gate on the update path — the half that is easy to leave open, because the
/// refusal that motivated this work would otherwise sit only where a resource is born.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_update_may_not_introduce_a_state_the_kind_does_not_carry(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("applic-update-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let id = create(&app, &token, &context_id, "concept", json!({})).await;

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "managed_meta": { "temper-status": "active" } }))
        .send()
        .await
        .expect("patch request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "a concept does not carry a goal's status"
    );
    let text = resp.text().await.expect("error body");
    assert!(
        text.contains("temper-status"),
        "the refusal must name the field: {text}"
    );
}

/// The gate must refuse a state of another kind, NOT every managed field. Without this the
/// two tests above pass against a gate that refuses everything, which would take the whole
/// system down rather than close the clause.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_kinds_own_state_and_the_provenance_trio_still_write(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("applic-ok-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let task = create(
        &app,
        &token,
        &context_id,
        "task",
        json!({ "temper-stage": "in-progress", "temper-mode": "build", "temper-effort": "small" }),
    )
    .await;
    let goal = create(
        &app,
        &token,
        &context_id,
        "goal",
        json!({ "temper-status": "active", "temper-seq": 3 }),
    )
    .await;
    // The provenance trio is declared by base.schema.json, so every kind carries it — and a
    // create stamps it regardless of type, which a gate reading only the doc-type schema would
    // refuse on all twelve non-task/goal kinds.
    let memory = create(&app, &token, &context_id, "memory", json!({})).await;

    for id in [&task, &goal, &memory] {
        let meta: serde_json::Value = app
            .client
            .get(app.url(&format!("/api/resources/{id}/meta")))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("GET meta failed")
            .json()
            .await
            .expect("meta JSON");
        assert!(
            meta["managed_meta"]["temper-provenance"].is_string(),
            "every kind carries the base-declared provenance trio; meta: {meta}"
        );
    }
}

/// **The restate half, and the remainder it rests on.**
///
/// A retype is how a resource comes to hold a state its (new) kind does not carry: the update
/// path writes only the keys the caller supplied, so a type change leaves the old kind's stored
/// properties in place. That is a **remaining hole in the clause** — closing it needs a way to
/// retract a property, which no door has — and it is asserted here rather than described, so
/// the day a delete gate lands this test fails and says so.
///
/// It is also the setup for the half that matters now: having reached that state, the resource
/// must still be restatable. `PUT /meta` states both tiers in full, so it echoes the stray back;
/// refusing it would wedge the resource with no exit.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retype_strands_a_state_the_new_kind_does_not_carry_and_it_stays_restatable(
    pool: PgPool,
) {
    let app = common::setup_test_app(pool).await;
    let email = format!("applic-restate-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let id = create(
        &app,
        &token,
        &context_id,
        "task",
        json!({ "temper-stage": "done" }),
    )
    .await;

    // Retype to a kind that carries no stage, mentioning no managed field.
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "type_to": "concept" }))
        .send()
        .await
        .expect("retype request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "the retype itself is permitted"
    );

    let meta: serde_json::Value = app
        .client
        .get(app.url(&format!("/api/resources/{id}/meta")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("GET meta failed")
        .json()
        .await
        .expect("meta JSON");
    assert_eq!(
        meta["managed_meta"]["temper-stage"], "done",
        "REMAINDER: a retype strands the old kind's state, because nothing can retract a \
         property row. When a delete gate closes this, change the test — do not delete it; \
         meta: {meta}"
    );

    // Having been stranded, the resource must remain restatable through the full-tier door.
    let resp = app
        .client
        .put(app.url(&format!("/api/resources/{id}/meta")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "resource_id": id,
            "managed_meta": { "temper-stage": "done", "temper-provenance": "user-created" },
            "open_meta": { "note": "restated" },
            "managed_hash": "",
            "open_hash": ""
        }))
        .send()
        .await
        .expect("meta restate failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("restate response JSON");
    assert_eq!(
        status, 200,
        "a stray field already stored may be restated — nothing can remove it, so refusing \
         would wedge the resource; body: {body}"
    );
}

/// The restate licence is scoped to the field actually stored. A resource holding one stray
/// must not thereby accept a different one — otherwise the first stray reopens the whole door.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_stored_stray_does_not_license_a_different_one(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("applic-scope-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let id = create(
        &app,
        &token,
        &context_id,
        "task",
        json!({ "temper-stage": "done" }),
    )
    .await;
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "type_to": "concept" }))
        .send()
        .await
        .expect("retype request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let resp = app
        .client
        .put(app.url(&format!("/api/resources/{id}/meta")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "resource_id": id,
            "managed_meta": { "temper-stage": "done", "temper-status": "active" },
            "open_meta": {},
            "managed_hash": "",
            "open_hash": ""
        }))
        .send()
        .await
        .expect("meta request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "storing temper-stage must not license introducing temper-status"
    );
}

/// The open tail, at the door rather than in the predicate.
///
/// A doc type the enum does not name has no schema to consult, so the gate must have **no
/// opinion** — not "carries nothing". Live resources sit on out-of-vocabulary types (26 of them
/// at the last count, and other deployments have their own), and every create stamps the
/// base-declared provenance trio. A gate that read "no schema" as "carries nothing" would refuse
/// every write to all of them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_out_of_vocabulary_doc_type_is_still_writable(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("applic-tail-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    // `kernel_landmark` is used verbatim rather than a made-up string because it is a real type
    // the substrate writes; a fictional one would pin the mechanism while leaving the case that
    // motivated it untested.
    let id = create(&app, &token, &context_id, "kernel_landmark", json!({})).await;

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "managed_meta": { "temper-provenance": "user-created" } }))
        .send()
        .await
        .expect("patch request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "a type with no schema carries no applicability opinion, so the write stands"
    );
}
