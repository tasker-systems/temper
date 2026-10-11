#![cfg(feature = "test-db")]
//! A read-only machine principal aimed at a write door is refused by the door's gate — `403` —
//! never a `500` (resource erasure 2c; `release-register/HISTORY.md`'s "Resource erasure 2c" row: "A
//! principal with no emitter to resolve (a read-only machine client) is refused by the gate (`403`)
//! on create and on these doors, where it got `500`").
//!
//! A registered machine client, seeded as `auth_seam_m2m_e2e` / `resource_facet_e2e_test` seed one,
//! carries NO `kb_entities` row: `resolve_machine_from_claims` looks the profile up and provisions
//! nothing, and `writes::resolve_emitter` is a `fetch_one` with no lazy creation. So on any door
//! that resolves the caller's emitter BEFORE its authorization gate, this principal's refusal turns
//! into an emitter-resolution failure — `ApiError::Internal`, a `500`. Every write door now
//! resolves the emitter only after its gate admits; this file is the HTTP witness for the doors the
//! facet e2e (`a_read_only_machine_principal_reads_facets_on_mcp_and_cannot_assert_one`) does not
//! reach: create (both create doors), delete, annotate, data-artifact commit and edge assert.
//!
//! The machine READS what it is refused: it is a `watcher` of the team that owns the context
//! (`contexts_readable_by_teams` admits every member; `context_authorable_by_profile` admits only
//! `owner`/`maintainer`/`member`), so the context resolves for it and the resource reads `200` — the
//! refusal below is the write gate's, not a visibility miss and not a shape error.
//!
//! Driven over plain HTTP with the machine's own `client_credentials` bearer, so the API adjudicates
//! the real token on the wire.

mod common;

use reqwest::Method;
use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use uuid::Uuid;

const MACHINE_CLIENT: &str = "write-floor-reader";

/// A registered, approved machine principal with no emitter entity. Returns its profile id.
async fn read_only_machine(pool: &PgPool) -> Uuid {
    let profile = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-write-floor-reader', 'agent-write-floor-reader', NULL, '{}')",
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
    common::approve(pool, profile).await;
    profile
}

/// A team owning a context, with `author` a `member` (authors it) and `watcher` a `watcher`
/// (reads it, cannot author it). Returns the context id.
async fn team_context(pool: &PgPool, author: Uuid, watcher: Uuid) -> Uuid {
    let team = Uuid::now_v7();
    let slug = format!("machine-floor-team-{}", &team.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(team)
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert team");
    let context = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_teams', $2, 'machine-floor', 'machine-floor')",
    )
    .bind(context)
    .bind(team)
    .execute(pool)
    .await
    .expect("insert team-owned context");
    for (profile, role) in [(author, "member"), (watcher, "watcher")] {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, $3::team_role)",
        )
        .bind(team)
        .bind(profile)
        .bind(role)
        .execute(pool)
        .await
        .expect("add team member");
    }
    context
}

/// A one-shot, bodiless ingest payload homed in `context` (no ONNX: no body, no chunks to embed).
fn ingest_payload(context: Uuid, slug: &str) -> IngestPayload {
    IngestPayload {
        idempotency_key: None,
        segmented: None,
        title: format!("Machine floor {slug}"),
        origin_uri: format!("test://machine-floor-{slug}-{}", Uuid::new_v4()),
        context_ref: context.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    }
}

/// `method path` with `token`: the status and the parsed body (`Null` when not JSON).
async fn send(
    app: &common::E2eTestApp,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = app
        .reqwest_client
        .request(method, app.url(path))
        .bearer_auth(token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.expect("door request");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("door body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Live resources homed in `context`.
async fn homed_count(pool: &PgPool, context: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = $1 AND r.is_active",
    )
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("homed count")
}

/// Edges touching `resource`, folded or not.
async fn edge_count(pool: &PgPool, resource: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges \
          WHERE (source_table = 'kb_resources' AND source_id = $1) \
             OR (target_table = 'kb_resources' AND target_id = $1)",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("edge count")
}

/// FAILS IF any of the six doors answers the read-only machine anything but the gate's `403
/// FORBIDDEN` — in particular the `500 INTERNAL_ERROR` an emitter resolved ahead of the gate
/// produces. The bite, per door: move that door's `writes::resolve_profile(..)` +
/// `writes::resolve_emitter(..)` pair in `db_backend.rs` above its gate — above
/// `self.check_context_authorable(..)` in `create_resource_unread` (both create doors), or above
/// `write_floor::modify_floor_in_tx(..)` in `delete_resource`, `annotate_resource`,
/// `commit_data_artifact` and `assert_relationship` — and that door answers `500`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_read_only_machine_principal_is_refused_403_not_500_on_every_write_door(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let human = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight")
        .id;
    let machine = read_only_machine(&pool).await;
    let machine_token = common::generate_machine_jwt(MACHINE_CLIENT);
    let context = team_context(&pool, human, machine).await;

    let resource = Uuid::from(
        app.client
            .ingest()
            .create(&ingest_payload(context, "subject"))
            .await
            .expect("the human authors a resource in the team context")
            .id,
    );

    // ── preconditions: the machine has no emitter, and reads what it will be refused ──────────
    let emitters: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_entities WHERE profile_id = $1")
            .bind(machine)
            .fetch_one(&pool)
            .await
            .expect("emitter count");
    assert_eq!(
        emitters, 0,
        "precondition: the machine has no emitter entity to resolve"
    );
    let (status, body) = send(
        &app,
        &machine_token,
        Method::GET,
        &format!("/api/resources/{resource}"),
        None,
    )
    .await;
    assert_eq!(
        status, 200,
        "precondition: the machine reads the resource (a watcher of its context); body: {body}"
    );

    // ── the doors ─────────────────────────────────────────────────────────────────────────────
    let doors: Vec<(&str, Method, String, Option<Value>)> = vec![
        (
            "POST /api/ingest (into a context it reads, cannot author)",
            Method::POST,
            "/api/ingest".to_string(),
            Some(serde_json::to_value(ingest_payload(context, "machine-create")).expect("body")),
        ),
        (
            "POST /api/resources (into a context it reads, cannot author)",
            Method::POST,
            "/api/resources".to_string(),
            Some(json!({
                "kb_context_id": context,
                "doc_type": "research",
                "origin_uri": format!("test://machine-floor-create-{}", Uuid::new_v4()),
                "title": "A create the gate must refuse",
            })),
        ),
        (
            "DELETE /api/resources/{id}",
            Method::DELETE,
            format!("/api/resources/{resource}"),
            None,
        ),
        (
            "POST /api/resources/{id}/provenance",
            Method::POST,
            format!("/api/resources/{resource}/provenance"),
            Some(json!({
                "sources": [{ "kind": "remote", "value": "https://example.com/machine-floor" }],
            })),
        ),
        (
            "POST /api/resources/{id}/artifacts",
            Method::POST,
            format!("/api/resources/{resource}/artifacts"),
            Some(json!({
                "kind": "machine-floor-probe",
                "intent": "current",
                "content": { "probe": true },
            })),
        ),
        (
            "POST /api/relationships (source it reads, cannot modify)",
            Method::POST,
            "/api/relationships".to_string(),
            Some(json!({
                "source": resource,
                "target": resource,
                "edge_kind": "leads_to",
                "polarity": "forward",
                "label": "machine-floor-probe",
                "weight": 1.0,
            })),
        ),
    ];

    for (name, method, path, body) in doors {
        let (status, answer) = send(&app, &machine_token, method, &path, body).await;
        assert_ne!(
            status, 500,
            "{name}: a refused machine must never answer 500; body: {answer}"
        );
        assert_ne!(
            answer["error"]["code"], "INTERNAL_ERROR",
            "{name}: never an internal error; body: {answer}"
        );
        assert_eq!(
            status, 403,
            "{name}: the door's gate refuses the machine; body: {answer}"
        );
        assert_eq!(
            answer["error"]["code"], "FORBIDDEN",
            "{name}: the gate's uniform deny; body: {answer}"
        );
    }

    // ── nothing landed ────────────────────────────────────────────────────────────────────────
    assert_eq!(
        homed_count(&pool, context).await,
        1,
        "neither create landed, and the delete did not tombstone the subject"
    );
    assert_eq!(
        edge_count(&pool, resource).await,
        0,
        "the refused edge assert wrote no edge"
    );
}
