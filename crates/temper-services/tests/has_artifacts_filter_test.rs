//! `filtered_visible_page`'s `has_artifacts` filter — the read surface for the ownership
//! partition clause.
//!
//! Three arms, failing in opposite directions (the `tag_filter_encoding_test` discipline):
//! an absent param must change nothing (a filter that silently narrows the default would be a
//! regression on every existing list caller), `true` and `false` must partition, and **ownership
//! is not liveness** — a folded artifact still means "owns", because the clause this serves says
//! "own at least one artifact and which own none" with no liveness qualifier. A build that
//! filtered on `NOT is_folded` satisfies the first two arms and fails only the third.
//!
//! The predicate sits under the same `resources_visible_to` join as every other filter, so a
//! caller who cannot read a resource gets no existence signal either way; that property is the
//! visibility join's own (tested at the gate), not restated here.

#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CommitDataArtifact, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceListParams;

/// Seed a substrate profile + a profile-owned `temper` context. Mirrors the inlined fixture in
/// `tag_filter_encoding_test.rs` / `list_page_query_count_test.rs`.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
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
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// Create one resource through the real write path, and return its id.
async fn mk(backend: &DbBackend, context: uuid::Uuid, slug: &str) -> uuid::Uuid {
    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: "note".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create");
    created.value.id.into()
}

/// Commit one artifact to a resource through the real write path.
async fn commit_artifact(backend: &DbBackend, resource: uuid::Uuid, kind: &str) {
    backend
        .commit_data_artifact(CommitDataArtifact {
            resource: ResourceId::from(resource),
            kind: kind.to_string(),
            kind_owner: None,
            intent: "current".to_string(),
            precedence: 0.0,
            content: serde_json::json!({ "n": 1 }),
            supersedes: vec![],
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("commit artifact");
}

/// The ids `filtered_visible_page` returns for one `has_artifacts` filter, scoped to the
/// test's context — the migrator boot-seeds visible system resources outside it, and the
/// clause this serves is per-context anyway.
async fn list_by_ownership(
    pool: &PgPool,
    principal: uuid::Uuid,
    context: uuid::Uuid,
    has_artifacts: Option<bool>,
) -> Vec<uuid::Uuid> {
    substrate_read::list_select(
        pool,
        ProfileId::from(principal),
        ResourceListParams {
            context_ref: Some(context.to_string()),
            has_artifacts,
            ..Default::default()
        },
    )
    .await
    .expect("list")
    .rows
    .into_iter()
    .map(|r| r.id.into())
    .collect()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_filter_partitions_ownership_and_an_absent_param_changes_nothing(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "has-artifacts@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let owner = mk(&backend, context, "owns").await;
    let empty = mk(&backend, context, "owns-none").await;
    commit_artifact(&backend, owner, "measurement").await;

    // Absent: both, unchanged — the regression arm for every existing list caller.
    let unfiltered = list_by_ownership(&pool, profile, context, None).await;
    assert_eq!(unfiltered.len(), 2);
    assert!(unfiltered.contains(&owner) && unfiltered.contains(&empty));

    let has = list_by_ownership(&pool, profile, context, Some(true)).await;
    assert_eq!(has, vec![owner]);

    let has_not = list_by_ownership(&pool, profile, context, Some(false)).await;
    assert_eq!(has_not, vec![empty]);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_folded_artifact_still_means_ownership(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "folded-owns@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let owner = mk(&backend, context, "owns-folded").await;
    let empty = mk(&backend, context, "owns-none").await;
    commit_artifact(&backend, owner, "measurement").await;

    // Arrange the stored state directly: the read must classify what IS stored, and how
    // `is_folded` became true is the write path's supersedes projection, tested by
    // temper-substrate's own suites. Setting it here makes "every artifact folded" reachable
    // without a superseding commit whose own liveness would defeat the arrangement.
    sqlx::query("UPDATE kb_data_artifacts SET is_folded = true WHERE resource_id = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .expect("fold");

    let has = list_by_ownership(&pool, profile, context, Some(true)).await;
    assert_eq!(
        has,
        vec![owner],
        "folded artifacts still count as ownership"
    );

    let has_not = list_by_ownership(&pool, profile, context, Some(false)).await;
    assert_eq!(has_not, vec![empty]);
}
