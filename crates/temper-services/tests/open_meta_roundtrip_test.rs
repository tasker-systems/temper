//! Regression guard for issue #307 — `open_meta` (the free-form open tier) must
//! round-trip through the `chunks_packed: None` create/update path (the shape the
//! MCP and CLI surfaces emit). Every prior create/update round-trip test seeded
//! `chunks_packed: Some(...)`, so this exact path was uncovered. Exercises the
//! real `DbBackend` (`create_resource`/`update_resource`) + the meta readback
//! (`substrate_read::get_meta_select`) the `--meta-only` / `GET .../meta`
//! projection uses.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CreateResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the
/// write path's `resolve_emitter` + visibility gate require). Mirrors the
/// temper-api `create_test_profile_with_context` fixture, inlined so this test
/// has no cross-crate test-harness dependency.
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

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_meta_round_trips_on_create_and_update(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "open-meta@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    // --- create: managed + open tiers on the same call, chunks_packed None ---
    let created = backend
        .create_resource(CreateResource {
            slug: "zz-open-meta-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "ZZ open_meta probe".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                provenance: Some("llm-discovered".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: Some(serde_json::json!({
                "marker": "TEST",
                "sub_marker": "999",
                "is_dropping": true
            })),
            goal: None,
            origin_uri: Some("test://open-meta-probe".to_string()),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::Mcp,
        })
        .await
        .expect("create")
        .value;

    let meta = substrate_read::get_meta_select(&pool, ProfileId::from(profile), created.id)
        .await
        .expect("get_meta after create");
    let open = meta.open_meta.expect("open_meta present after create");
    assert_eq!(open.get("marker"), Some(&serde_json::json!("TEST")));
    assert_eq!(open.get("sub_marker"), Some(&serde_json::json!("999")));
    assert_eq!(open.get("is_dropping"), Some(&serde_json::json!(true)));
    // The managed sibling survives too (proves both tiers persist on one call).
    assert_eq!(
        meta.managed_meta.and_then(|m| m.provenance),
        Some("llm-discovered".to_string())
    );

    // --- update: a meta-only PATCH (body None, managed None) sets a new open key ---
    backend
        .update_resource(UpdateResource {
            resource: created.id,
            title: None,
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: Some(serde_json::json!({"reviewed_by": "qa"})),
            goal: None,
            move_to: None,
            context_ref: None,
            act: ActContext::default(),
            origin: Surface::Mcp,
        })
        .await
        .expect("update");

    let meta2 = substrate_read::get_meta_select(&pool, ProfileId::from(profile), created.id)
        .await
        .expect("get_meta after update");
    let open2 = meta2.open_meta.expect("open_meta present after update");
    assert_eq!(
        open2.get("reviewed_by"),
        Some(&serde_json::json!("qa")),
        "open_meta key set on update must round-trip"
    );
}
