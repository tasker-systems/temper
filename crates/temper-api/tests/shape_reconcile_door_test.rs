#![cfg(feature = "test-db")]
//! The shape-reconcile drain runs in production through its cron door, not only in tests.
//!
//! A shape declared over an artifact committed before it leaves that artifact
//! `declared_not_yet_checked` and queues a reconcile job. Nothing but `/api/shape/reconcile`
//! drains that job in a deployment, so this test drives the door through the real router, on the
//! pinned cron host with the dispatch secret, and reads the verdict it wrote. A door that answered
//! without running the drain would fail here.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use temper_services::services::shape_service::{self, DeclareShapeServiceParams};
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode};
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams};
use uuid::Uuid;

const DOOR: &str = "/api/shape/reconcile";
const PINNED: &str = "temper-cloud-a1b2c3d4e-team.vercel.app";
const SECRET: &str = "shape-reconcile-door-test-secret-0123456789";

async fn emitter_of(pool: &PgPool, profile: Uuid) -> EntityId {
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@web'",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("the fixture mints a web emitter");
    EntityId::from(id)
}

// FAILS IF: the door answers without running the drain, or the drain it runs does not verdict an
// artifact committed before its shape was declared.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_door_verdicts_an_artifact_committed_before_its_shape(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| {
        c.cron_host = Some(PINNED.to_string());
        c.embed_dispatch_secret = Some(SECRET.to_string());
    })
    .await;
    let email = format!("shape-door-{}@example.com", Uuid::new_v4());
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let emitter = emitter_of(&app.pool, owner).await;
    let home = AnchorRef::context(ContextId::from(context));
    let profile = ProfileId::from(owner);

    let resource = writes::create_resource_deferred_with(
        &app.pool,
        CreateParams {
            idempotency_key: None,
            title: "Door subject",
            origin_uri: "test://shape-reconcile-door/subject",
            body: "door subject",
            doc_type: "research",
            home,
            owner: profile,
            originator: profile,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create");
    let artifact = writes::commit_data_artifact(
        &app.pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(resource.uuid()),
            kind: "measurement",
            kind_owner: None,
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &json!({ "value": 42 }),
            supersedes: &[],
            emitter,
        },
    )
    .await
    .expect("commit")
    .uuid();
    shape_service::declare_shape(
        &app.pool,
        DeclareShapeServiceParams {
            home,
            kind: "measurement",
            kind_owner: None,
            schema: &json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            }),
            enforcement: EnforcementMode::Advisory,
            principal: profile,
            emitter,
        },
    )
    .await
    .expect("declare");

    let response = app
        .client
        .post(app.url(DOOR))
        .header(reqwest::header::HOST, PINNED)
        .bearer_auth(SECRET)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let summary: Value = response.json().await.expect("a JSON summary");

    assert_eq!(summary["verdicts_written"], json!(1), "{summary}");
    assert_eq!(summary["completed"], json!(1), "{summary}");
    let satisfied: Option<bool> = sqlx::query_scalar(
        "SELECT satisfied FROM kb_data_artifact_verdicts WHERE artifact_id = $1",
    )
    .bind(artifact)
    .fetch_optional(&app.pool)
    .await
    .expect("read the verdict");
    assert_eq!(
        satisfied,
        Some(false),
        "a number where the shape asks a string is a verdict, not satisfied"
    );
}
