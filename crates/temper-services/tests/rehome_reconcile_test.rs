//! Integration test — `resource_rehome` enqueues a shape-reconcile job for the destination
//! home (task `01a02a98`, filed from the shape registry build's consolidated review).
//!
//! The staleness triple (`shape_id`, `shape_version`, `content_hash`) already makes rehomed
//! artifacts read as `DeclaredNotYetChecked` — this test verifies the enqueue that moves them
//! to a real verdict actually happens.
#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_services::backend::DbBackend;
use temper_services::services::shape_service;
use temper_services::services::shape_service::DeclareShapeServiceParams;
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode};
use temper_substrate::writes::{self, CommitDataArtifactParams};
use temper_workflow::operations::{
    Backend, BodyUpdate, CreateResource, MoveSpec, Surface, UpdateResource,
};
use temper_workflow::types::managed_meta::ManagedMeta;

async fn seed_profile(pool: &PgPool, email: &str) -> uuid::Uuid {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    profile_id
}

async fn seed_context(pool: &PgPool, owner: uuid::Uuid, slug: &str) -> uuid::Uuid {
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,$3,$3)",
    )
    .bind(context_id)
    .bind(owner)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed context");
    context_id
}

fn create_cmd(home: HomeAnchor, slug: &str) -> CreateResource {
    let content = format!("body of {slug}");
    CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home,
        title: slug.to_string(),
        body: Some(BodyUpdate {
            content,
            content_hash: None,
            chunks_packed: None,
            sources: Vec::new(),
            content_block: None,
        }),
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal: None,
        origin_uri: Some(format!("test://{slug}")),
        chunks_packed: None,
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

fn rehome_cmd(resource: uuid::Uuid, dest: uuid::Uuid) -> UpdateResource {
    UpdateResource {
        open_meta_add: None,
        resource: ResourceId::from(resource),
        title: None,
        slug: None,
        body: None,
        managed_meta: None,
        open_meta: None,
        goal: None,
        move_to: Some(MoveSpec {
            context_to: Some(ContextId::from(dest)),
            type_to: None,
        }),
        context_ref: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

async fn shape_reconcile_job_count(pool: &PgPool, context_id: uuid::Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs \
         WHERE context_id = $1 \
           AND persona = 'shape' \
           AND dispatch_type = 'shape-reconcile' \
           AND status IN ('pending', 'in_progress', 'waiting_for_retry')",
    )
    .bind(context_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Rehoming a resource into a context that has a declared shape enqueues a shape-reconcile
/// job for the destination home. The job is what moves rehomed artifacts from
/// `DeclaredNotYetChecked` to a real verdict.
#[sqlx::test(migrations = "../../migrations")]
async fn rehoming_a_resource_enqueues_shape_reconcile(pool: PgPool) {
    let owner = seed_profile(&pool, "rehome-owner@example.com").await;
    let emitter_entity: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 LIMIT 1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap();
    let ctx_a = seed_context(&pool, owner, "ctx-a").await;
    let ctx_b = seed_context(&pool, owner, "ctx-b").await;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));

    // Create a resource in ctx_a.
    let resource = backend
        .create_resource(create_cmd(
            HomeAnchor::Context(ContextId::from(ctx_a)),
            "res-a",
        ))
        .await
        .expect("create resource")
        .value
        .id
        .uuid();

    // Commit an artifact owned by the resource, in ctx_a.
    let kind = "measurement";
    let content = serde_json::json!({"value": "42"});
    writes::commit_data_artifact(
        &pool,
        CommitDataArtifactParams {
            resource: ResourceId::from(resource),
            kind,
            kind_owner: None,
            intent: ArtifactIntent::Current,
            precedence: 0.0,
            content: &content,
            supersedes: &[],
            emitter: EntityId::from(emitter_entity),
        },
    )
    .await
    .expect("commit artifact");

    // Declare a shape in ctx_b (the destination). Needs a resource homed in ctx_b for
    // kind_owner defaulting to resolve.
    backend
        .create_resource(create_cmd(
            HomeAnchor::Context(ContextId::from(ctx_b)),
            "dummy-b",
        ))
        .await
        .expect("create dummy resource in ctx_b");

    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {"value": {"type": "string"}},
        "required": ["value"]
    });

    shape_service::declare_shape(
        &pool,
        DeclareShapeServiceParams {
            home: AnchorRef::context(ContextId::from(ctx_b)),
            kind,
            kind_owner: None,
            schema: &schema,
            enforcement: EnforcementMode::Advisory,
            principal: ProfileId::from(owner),
            emitter: EntityId::from(emitter_entity),
        },
    )
    .await
    .expect("declare shape in ctx_b");

    // The declare itself enqueues a reconcile job for ctx_b. Consume it so the baseline is clean.
    shape_service::reconcile_anchor(&pool, HomeAnchor::Context(ContextId::from(ctx_b)))
        .await
        .expect("drain declare's job");

    assert_eq!(
        shape_reconcile_job_count(&pool, ctx_b).await,
        0,
        "baseline: no shape-reconcile jobs for ctx_b after draining"
    );

    // Rehome the resource from ctx_a to ctx_b.
    backend
        .update_resource(rehome_cmd(resource, ctx_b))
        .await
        .expect("rehome resource");

    // The rehome must have enqueued a shape-reconcile job for ctx_b.
    assert_eq!(
        shape_reconcile_job_count(&pool, ctx_b).await,
        1,
        "rehome must enqueue exactly one shape-reconcile job for the destination home"
    );

    // Run the reconciler — it should verdict the rehomed artifact.
    let count = shape_service::reconcile_anchor(&pool, HomeAnchor::Context(ContextId::from(ctx_b)))
        .await
        .expect("reconcile after rehome");

    assert_eq!(
        count, 1,
        "exactly one verdict must be written — the rehomed artifact"
    );
}
