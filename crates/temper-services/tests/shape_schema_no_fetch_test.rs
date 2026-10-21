//! Integration test — a shape's schema is compiled without fetching, at declare and at reconcile
//! (task `01a15661-8460-739d-9ed3-f1e8ccbf9b69`).
//!
//! The substrate's `writes::declare_shape` does not compile, so a shape whose schema needs a fetch
//! is stored here through it: that is the state of a shape declared before the service's declare
//! compiled. Tests that touch such a schema also read the connection count of a loopback listener
//! the schema's `$ref` names. That count guards against a regression; it is not the fetch witness,
//! because a fetching compiler's blocking fetch panics inside this async runtime before it
//! connects. The witnesses are the synchronous `shape_schema_no_fetch` tests in `temper-substrate`
//! and the `audit-shape-schema-compile.sh` tripwire. What these tests pin is the behaviour: the
//! refusal, the absence of writes, and reconcile's skip.
#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_services::error::ApiError;
use temper_services::services::shape_service::{self, DeclareShapeServiceParams};
use temper_substrate::events::EventContext;
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, ShapeState};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, DeclareShapeParams};
use uuid::Uuid;

// The counting listener is shared with temper-substrate's tests, by path, so the two suites agree
// on what a connection is.
#[path = "../../temper-substrate/tests/common/fetch_probe.rs"]
mod fetch_probe;

use fetch_probe::FetchProbe;

// ── seeding ──────────────────────────────────────────────────────────────────────────────────

struct World {
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    resource: Uuid,
}

/// One context-homed resource owned by `system`, with an emitter entity for the writes.
async fn seed_world(pool: &PgPool, slug: &str) -> World {
    bootseed::seed_system(pool).await.unwrap();
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'test-emitter') RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap();
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
    let resource = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "subject",
            origin_uri: "subject",
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(ContextId::from(context)),
            owner: ProfileId::from(owner),
            originator: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    World {
        owner,
        emitter,
        context,
        resource: resource.uuid(),
    }
}

fn string_value_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    })
}

/// A schema that cannot compile without a fetch: its `$ref` names the probe's listener.
fn remote_ref_schema(probe: &FetchProbe) -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": probe.url("shape.json"),
    })
}

fn service_params<'a>(
    w: &World,
    principal: Uuid,
    kind: &'a str,
    schema: &'a serde_json::Value,
) -> DeclareShapeServiceParams<'a> {
    DeclareShapeServiceParams {
        home: AnchorRef::context(ContextId::from(w.context)),
        kind,
        kind_owner: None,
        schema,
        enforcement: EnforcementMode::Advisory,
        principal: ProfileId::from(principal),
        emitter: EntityId::from(w.emitter),
    }
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

// ── declare ──────────────────────────────────────────────────────────────────────────────────

/// A schema that needs a fetch is refused when it is declared: the typed refusal carries the fixed
/// sentence (no URI), no shape row and no reconcile job is written, and nothing is fetched. The
/// same owner then declares a valid 2020-12 schema, so the refusal is about the schema alone.
#[sqlx::test(migrations = "../../migrations")]
async fn declaring_a_schema_that_needs_a_fetch_is_refused_and_writes_nothing(pool: PgPool) {
    let w = seed_world(&pool, "declare-remote-ref").await;
    let probe = FetchProbe::bind();
    let remote = remote_ref_schema(&probe);

    let err =
        shape_service::declare_shape(&pool, service_params(&w, w.owner, "measurement", &remote))
            .await
            .expect_err("a schema that needs a fetch must be refused at declare");

    let message = match err {
        ApiError::DataArtifactRefusal(message) => message,
        other => panic!("the refusal must be the typed data-artifact refusal, got {other:?}"),
    };
    assert!(
        message.contains("cannot be compiled") && message.contains("never fetched"),
        "the refusal must carry the fixed sentence, got: {message}"
    );
    assert!(
        !message.contains("127.0.0.1") && !message.contains(&probe.port().to_string()),
        "the refusal must not echo the URI the schema names, got: {message}"
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM kb_data_artifact_shapes").await,
        0,
        "no shape row is written by a refused declare"
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM kb_workflow_jobs WHERE dispatch_type = 'shape-reconcile'"
        )
        .await,
        0,
        "no reconcile job is enqueued by a refused declare"
    );
    assert_eq!(probe.connections(), 0, "the declare must not fetch");

    let valid = string_value_schema();
    shape_service::declare_shape(&pool, service_params(&w, w.owner, "measurement", &valid))
        .await
        .expect("a self-contained 2020-12 schema still declares");
    assert_eq!(
        count(&pool, "SELECT count(*) FROM kb_data_artifact_shapes").await,
        1
    );
}

/// The authority gate runs first: a principal who cannot author the home is answered `Forbidden`
/// whatever the schema says, so the compile refusal teaches only an author.
#[sqlx::test(migrations = "../../migrations")]
async fn a_stranger_declaring_an_uncompilable_schema_is_forbidden_not_taught(pool: PgPool) {
    let w = seed_world(&pool, "declare-gate-first").await;
    let probe = FetchProbe::bind();
    let remote = remote_ref_schema(&probe);
    let stranger: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('shape-stranger', 'shape-stranger') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let err =
        shape_service::declare_shape(&pool, service_params(&w, stranger, "measurement", &remote))
            .await
            .expect_err("a stranger must be refused");

    assert!(
        matches!(err, ApiError::Forbidden),
        "the gate answers before the compile does, got {err:?}"
    );
}

// ── reconcile ────────────────────────────────────────────────────────────────────────────────

/// Reconcile over a stored shape that needs a fetch does not connect, writes no verdict for its
/// artifacts (they stay `declared_not_yet_checked`), and does not fail the anchor: a sibling
/// family's artifact in the same home is still verdicted and the job completes.
#[sqlx::test(migrations = "../../migrations")]
async fn reconcile_over_a_stored_remote_ref_shape_writes_no_verdict_and_does_not_connect(
    pool: PgPool,
) {
    let w = seed_world(&pool, "reconcile-remote-ref").await;
    let probe = FetchProbe::bind();
    let resource = ResourceId::from(w.resource);
    let content = serde_json::json!({"value": "42"});

    // Two families, each with one artifact committed before any shape is declared.
    let mut artifacts = Vec::new();
    for kind in ["measurement", "reading"] {
        let id = writes::commit_data_artifact(
            &pool,
            CommitDataArtifactParams {
                resource,
                kind,
                kind_owner: None,
                intent: ArtifactIntent::Current,
                precedence: 0.0,
                content: &content,
                supersedes: &[],
                emitter: EntityId::from(w.emitter),
            },
        )
        .await
        .unwrap();
        artifacts.push(id.uuid());
    }
    let (measurement_artifact, reading_artifact) = (artifacts[0], artifacts[1]);

    // Both families get a valid shape through the service (which enqueues the reconcile job);
    // then `measurement` is amended through the substrate write to a schema that needs a fetch.
    let valid = string_value_schema();
    for kind in ["measurement", "reading"] {
        shape_service::declare_shape(&pool, service_params(&w, w.owner, kind, &valid))
            .await
            .unwrap();
    }
    writes::declare_shape(
        &pool,
        DeclareShapeParams {
            home: AnchorRef::context(ContextId::from(w.context)),
            kind: "measurement",
            kind_owner: None,
            schema: &remote_ref_schema(&probe),
            enforcement: EnforcementMode::Advisory,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .unwrap();

    let written =
        shape_service::reconcile_anchor(&pool, HomeAnchor::Context(ContextId::from(w.context)))
            .await
            .expect("one uncompilable shape must not fail the anchor's reconcile");

    assert_eq!(
        written, 1,
        "exactly one verdict: the sibling family's artifact"
    );
    assert_eq!(probe.connections(), 0, "reconcile must not fetch");
    let verdict_for = |artifact: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT satisfied FROM kb_data_artifact_verdicts WHERE artifact_id = $1",
            )
            .bind(artifact)
            .fetch_optional(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        verdict_for(reading_artifact).await,
        Some(true),
        "the compilable family is verdicted"
    );
    assert_eq!(
        verdict_for(measurement_artifact).await,
        None,
        "the uncompilable family's artifact gets no verdict"
    );
    let state = temper_substrate::readback::artifacts_for_resource(
        &pool,
        ProfileId::from(w.owner),
        resource,
        Some("measurement"),
        None,
        false,
    )
    .await
    .unwrap()
    .first()
    .map(|a| a.shape_state);
    assert_eq!(
        state,
        Some(ShapeState::DeclaredNotYetChecked),
        "the artifact under the uncompilable shape reads as not yet checked"
    );
    let statuses: Vec<String> = sqlx::query_scalar(
        "SELECT status::text FROM kb_workflow_jobs \
          WHERE context_id = $1 AND persona = 'shape' AND dispatch_type = 'shape-reconcile'",
    )
    .bind(w.context)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        statuses,
        vec!["done".to_owned()],
        "the anchor's job completes; it is not left in flight for the reaper"
    );
}
