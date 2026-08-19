//! A facet histogram must not be filtered by its own predicate.
//!
//! `filtered_visible_page` applied the doc-type predicate in SQL and then built the histogram from
//! what came back, so `facets.doc_type` could only ever have the single key the caller had already
//! selected. That is the one shape a browse UI cannot use: the counts that would justify offering
//! the OTHER kinds are exactly the counts the filter removed, so no alternative is showable and
//! none is reachable. The fix moves the `doc_type_name`/`stage`/`status` predicates out of SQL and
//! into the existing Rust filter step, where each histogram can be computed over the set narrowed
//! by the other two.
//!
//! What this test pins is therefore a *negative*: each histogram counts options the caller did not
//! select. `total` stays fully filtered — it describes the page; the histograms describe its
//! neighbourhood, and the two must not be confused. The third case pins that `doc_type_name`
//! accepts a CSV and unions it (a resource has exactly one doc type, so ANDing could only match
//! nothing).
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceListParams;

/// Seed a substrate profile + a profile-owned `temper` context. Mirrors the inlined fixture in
/// `list_page_query_count_test.rs` / `open_meta_roundtrip_test.rs` / `segmented_backend_test.rs`.
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

/// Create one resource of `doctype` carrying `managed`, homed in `context`.
///
/// The managed tier is what lands in `kb_resource_workflow_props` (`temper-stage`/`temper-status`),
/// which is where the stage and status histograms read from — so the seed has to go through the
/// real create path rather than inserting properties directly, or it would be testing a shape the
/// write path does not produce.
async fn seed_resource(
    backend: &DbBackend,
    context: uuid::Uuid,
    slug: &str,
    doctype: &str,
    managed: ManagedMeta,
) {
    backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: doctype.to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: managed,
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
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn facet_histograms_exclude_their_own_predicate(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "list-facets@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    // 3 tasks (stages backlog, backlog, done), 2 goals (active, completed), 1 research. The
    // migration chain also seeds the L0 kernel's telos resource, whose doc_type is
    // `cogmap_charter` and which carries neither stage nor status — so it can add a key to the
    // doc_type histogram but cannot perturb any count asserted below.
    for (slug, stage) in [
        ("zz-facet-task-a", "backlog"),
        ("zz-facet-task-b", "backlog"),
        ("zz-facet-task-c", "done"),
    ] {
        seed_resource(
            &backend,
            context,
            slug,
            "task",
            ManagedMeta {
                stage: Some(stage.to_string()),
                ..ManagedMeta::default()
            },
        )
        .await;
    }
    for (slug, status) in [
        ("zz-facet-goal-a", "active"),
        ("zz-facet-goal-b", "completed"),
    ] {
        seed_resource(
            &backend,
            context,
            slug,
            "goal",
            ManagedMeta {
                status: Some(status.to_string()),
                ..ManagedMeta::default()
            },
        )
        .await;
    }
    seed_resource(
        &backend,
        context,
        "zz-facet-research-a",
        "research",
        ManagedMeta::default(),
    )
    .await;

    // A doc-type filter must NOT shrink the doc-type histogram — the defect this change fixes.
    let params = ResourceListParams {
        doc_type_name: Some("task".to_string()),
        ..Default::default()
    };
    let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
        .await
        .expect("list");
    assert_eq!(page.total, 3, "total IS filtered");
    assert_eq!(page.facets.doc_type.get("task"), Some(&3));
    assert_eq!(
        page.facets.doc_type.get("goal"),
        Some(&2),
        "the doc_type histogram must exclude its own predicate"
    );
    assert_eq!(
        page.facets.doc_type.get("research"),
        Some(&1),
        "every other kind stays visible and reachable"
    );

    // The stage histogram excludes its own predicate but respects doc_type.
    let params = ResourceListParams {
        doc_type_name: Some("task".to_string()),
        stage: Some("done".to_string()),
        ..Default::default()
    };
    let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
        .await
        .expect("list");
    assert_eq!(page.total, 1, "total reflects BOTH filters");
    assert_eq!(
        page.facets.stage.get("backlog"),
        Some(&2),
        "the stage histogram excludes its own predicate"
    );
    assert_eq!(page.facets.stage.get("done"), Some(&1));
    assert!(
        !page.facets.stage.contains_key("active"),
        "stage histogram is scoped by doc_type, which is NOT its own predicate"
    );

    // The status histogram excludes its own predicate but respects doc_type. Same invariant as the
    // stage case one dimension over — asserted separately because a histogram that silently
    // applied its own predicate would still satisfy every assertion above.
    let params = ResourceListParams {
        doc_type_name: Some("goal".to_string()),
        status: Some("active".to_string()),
        ..Default::default()
    };
    let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
        .await
        .expect("list");
    assert_eq!(page.total, 1, "total reflects BOTH filters");
    assert_eq!(
        page.facets.status.get("completed"),
        Some(&1),
        "the status histogram excludes its own predicate — the unselected option keeps its count"
    );
    assert_eq!(page.facets.status.get("active"), Some(&1));
    assert!(
        !page.facets.status.contains_key("backlog"),
        "status histogram is scoped by doc_type, which is NOT its own predicate"
    );

    // CSV multi-select.
    let params = ResourceListParams {
        doc_type_name: Some("task,goal".to_string()),
        ..Default::default()
    };
    let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
        .await
        .expect("list");
    assert_eq!(page.total, 5, "CSV selects the union");
}
