#![cfg(feature = "test-db")]
//! **Asking artifacts through `POST /api/query`: who can phrase the question** (spec
//! `2026-10-19-asking-artifacts-by-what-they-are` §3.5, §6.3).
//!
//! Through the HTTP door because two of the properties are about what reaches a caller over it: an
//! unreadable shape and an absent one answer with the same status and the same body, and a
//! principal who reads a resource by direct grant but cannot read its home is refused the question
//! while the artifact reads still serve them.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, DeclareShapeParams};
use uuid::Uuid;

/// A's world: A's `temper` context holding resource R, A's `measurement` shape declared there, and
/// one artifact on R that the ask matches.
struct World {
    app: common::TestApp,
    owner_token: String,
    other_token: String,
    other: Uuid,
    owner: Uuid,
    resource: Uuid,
    artifact: Uuid,
    shape: Uuid,
}

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

async fn world(pool: PgPool) -> World {
    let app = common::setup_test_app(pool).await;
    let owner_email = format!("ask-owner-{}@example.com", Uuid::new_v4());
    let other_email = format!("ask-other-{}@example.com", Uuid::new_v4());
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&app.pool, &owner_email).await;
    let (other, _) =
        common::fixtures::create_test_profile_with_context(&app.pool, &other_email).await;
    let owner_token = common::generate_test_jwt(&format!("test|{owner}"), &owner_email);
    let other_token = common::generate_test_jwt(&format!("test|{other}"), &other_email);

    let emitter = emitter_of(&app.pool, owner).await;
    let home = AnchorRef::context(ContextId::from(context));
    let profile = ProfileId::from(owner);
    let resource = writes::create_resource_deferred_with(
        &app.pool,
        CreateParams {
            idempotency_key: None,
            title: "Kestrel latency",
            origin_uri: "test://query-artifact-ask/kestrel",
            body: "kestrel latency measurements",
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
    .expect("create")
    .uuid();
    let shape = writes::declare_shape(
        &app.pool,
        DeclareShapeParams {
            home,
            kind: "measurement",
            kind_owner: Some(KindOwner::Profile(owner)),
            schema: &json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "unit": { "type": "string" } }
            }),
            enforcement: EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .expect("declare")
    .uuid();
    let artifact = writes::commit_data_artifact(
        &app.pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(resource),
            kind: "measurement",
            kind_owner: Some(KindOwner::Profile(owner)),
            intent: ArtifactIntent::Member,
            precedence: 0.0,
            content: &json!({ "unit": "ms" }),
            supersedes: &[],
            emitter,
        },
    )
    .await
    .expect("commit")
    .uuid();

    World {
        app,
        owner_token,
        other_token,
        other,
        owner,
        resource,
        artifact,
        shape,
    }
}

/// A selection asking `unit` contains `"ms"` of `shape`, piped into a returned `find-exact`.
fn plan(shape: Uuid) -> Value {
    json!({
        "stages": [
            { "name": "sel", "act": "find-resources-with",
              "artifact_filter": [
                  { "shape": shape,
                    "predicates": [ { "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } } ] }
              ] },
            { "name": "hits", "act": "find-exact", "intention": { "query": "kestrel" },
              "inputs": [ { "from": "upstream", "as": "bound", "stage": "sel" } ] }
        ],
        "outcome": { "returns": [ { "stage": "hits", "with": [] } ] }
    })
}

async fn post(w: &World, token: &str, body: &Value) -> (StatusCode, Value) {
    let resp = w
        .app
        .client
        .post(w.app.url("/api/query"))
        .header("Authorization", format!("Bearer {token}"))
        .json(body)
        .send()
        .await
        .expect("POST /api/query");
    let status = resp.status();
    (status, resp.json().await.expect("a JSON body"))
}

async fn get(w: &World, token: &str, path: &str) -> (StatusCode, Value) {
    let resp = w
        .app
        .client
        .get(w.app.url(path))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("GET");
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

fn returned_ids(body: &Value) -> Vec<String> {
    body["returned"]["hits"]["produced"]["hits"]
        .as_array()
        .map(|hits| {
            hits.iter()
                .filter_map(|h| h["resource"]["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// **A shape the caller cannot read and a shape that does not exist answer identically: the same
/// status and, byte for byte, the same body.** The other principal cannot read A's context, so A's
/// shape is unreadable to them; a fresh uuid names nothing. Both are `400 PLAN_REFUSED` with one
/// refusal, `shape_not_askable` at the asking stage, and nothing in either body says which. The
/// owner asking the same plan is answered — the control that the shape is askable at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unreadable_shape_and_an_absent_one_answer_the_same(pool: PgPool) {
    let w = world(pool).await;

    let (owner_status, owner_body) = post(&w, &w.owner_token, &plan(w.shape)).await;
    assert_eq!(owner_status, StatusCode::OK, "{owner_body}");
    assert_eq!(returned_ids(&owner_body), vec![w.resource.to_string()]);

    let (unreadable_status, unreadable) = post(&w, &w.other_token, &plan(w.shape)).await;
    let (absent_status, absent) = post(&w, &w.other_token, &plan(Uuid::now_v7())).await;
    assert_eq!(unreadable_status, StatusCode::BAD_REQUEST, "{unreadable}");
    assert_eq!(absent_status, unreadable_status);
    assert_eq!(absent, unreadable, "the two bodies are one");
    assert_eq!(
        unreadable["error"]["code"],
        temper_core::error::PLAN_REFUSED_CODE
    );
    let refusals = unreadable["error"]["details"]["refusals"]
        .as_array()
        .expect("the refusals ride the details");
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0]["reason"], "shape_not_askable");
    assert_eq!(refusals[0]["stage"], "sel");
    assert!(
        !unreadable.to_string().contains(&w.shape.to_string()),
        "the refusal never names the shape it refused"
    );
}

/// **A principal who reads a resource by direct grant, but not its home, cannot ask the shape that
/// governs its artifacts — and still lists and shows those artifacts.** The grant puts R in the
/// other principal's visible set; the shape lives in a home they cannot read, so `POST /api/query`
/// refuses the ask `shape_not_askable`, while `GET /api/resources/{id}/artifacts` and
/// `GET /api/data-artifacts/{id}` serve the artifact. Their data stays reachable; the question is
/// not theirs to phrase in a vocabulary they cannot read.
///
/// The precondition is asserted, not assumed: before the grant, the artifact reads refuse them, so
/// what serves them afterwards is the grant.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_direct_grant_reads_the_artifacts_and_cannot_ask_their_shape(pool: PgPool) {
    let w = world(pool).await;
    let list = format!("/api/resources/{}/artifacts", w.resource);
    let show = format!("/api/data-artifacts/{}", w.artifact);

    let (before, _) = get(&w, &w.other_token, &show).await;
    assert_eq!(
        before,
        StatusCode::NOT_FOUND,
        "the precondition: not visible yet"
    );

    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(w.resource)
    .bind(w.other)
    .bind(w.owner)
    .execute(&w.app.pool)
    .await
    .expect("grant read on the resource");

    let (listed, artifacts) = get(&w, &w.other_token, &list).await;
    assert_eq!(listed, StatusCode::OK, "{artifacts}");
    assert!(
        artifacts
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x["artifact_id"] == w.artifact.to_string())),
        "list serves the artifact: {artifacts}"
    );
    let (shown, artifact) = get(&w, &w.other_token, &show).await;
    assert_eq!(shown, StatusCode::OK, "{artifact}");

    let (status, body) = post(&w, &w.other_token, &plan(w.shape)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let refusals = body["error"]["details"]["refusals"]
        .as_array()
        .expect("the refusals ride the details");
    assert_eq!(
        refusals
            .iter()
            .map(|r| r["reason"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["shape_not_askable"],
        "{body}"
    );
}
