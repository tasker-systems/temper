//! Regression guard: a context's **display name** may legally contain spaces and capitals, and
//! that must not block writes to resources homed in it.
//!
//! `base.schema.json` requires `temper-context` and constrains it to `^[a-z0-9][a-z0-9-]*$` — a
//! **slug** pattern — so whatever the write path injects there must be slug-shaped. The update
//! path used to inject `ResourceRowParity::context_name`, the *display* name. A context named
//! `working agreements` (slug `working-agreements`) therefore 400'd **every managed-tier write to
//! every resource in it**, for every doc type, since `temper-context` is required in the shared
//! base schema rather than in any one doc-type schema.
//!
//! Create never saw it: its token is the raw home UUID, which matches the slug pattern by accident.
//! That asymmetry is why the defect survived — one shared validation field carrying a UUID on one
//! path and a display name on the other.
//!
//! Observed in production 2026-08-02 on `@me/working-agreements`; 43 of 107 retitles failed.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_workflow::operations::{Backend, CreateResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;

use temper_services::backend::DbBackend;

/// Seed a profile plus a context whose **name and slug deliberately differ**: the name carries a
/// space (illegal for the schema's `temper-context` pattern), the slug does not.
async fn seed_profile_with_named_context(
    pool: &PgPool,
    email: &str,
    slug: &str,
    display_name: &str,
) -> (uuid::Uuid, uuid::Uuid) {
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
         VALUES ($1,'kb_profiles',$2,$3,$4)",
    )
    .bind(context_id)
    .bind(profile_id)
    .bind(slug)
    .bind(display_name)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// The bite: the display name contains a space, so the pre-fix update path fed
/// `"working agreements"` into a slug-patterned field and the retitle 400'd.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_spaced_context_display_name_does_not_block_a_managed_tier_update(pool: PgPool) {
    let (profile, context) = seed_profile_with_named_context(
        &pool,
        "spaced-ctx@example.com",
        "working-agreements",
        "working agreements", // ← the space is the whole point
    )
    .await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "zz-spaced-context-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "before".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://spaced-context".to_string()),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::Mcp,
        })
        .await
        .expect("create must succeed — create's token is the home UUID, which was never the bug")
        .value;

    // A title change is a MANAGED-tier write: it runs the managed_meta validation pipeline, which
    // is where `temper-context` is injected and pattern-checked. This is the call that 400'd.
    let updated = backend
        .update_resource(UpdateResource {
            open_meta_add: None,
            resource: created.id,
            title: Some("after".to_string()),
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: None,
            goal: None,
            move_to: None,
            context_ref: None,
            act: ActContext::default(),
            origin: Surface::Mcp,
        })
        .await;

    assert!(
        updated.is_ok(),
        "a managed-tier update must not fail because the context's DISPLAY NAME is not \
         slug-shaped; got: {:?}",
        updated.err()
    );
}

/// The control. A slug-shaped display name passed before the fix too, so on its own it proves
/// nothing — it is here so a future change that breaks the ordinary path is distinguishable from
/// one that breaks only the spaced-name path.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_slug_shaped_context_display_name_still_works(pool: PgPool) {
    let (profile, context) = seed_profile_with_named_context(
        &pool,
        "plain-ctx@example.com",
        "working-agreements",
        "working-agreements",
    )
    .await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "zz-plain-context-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "before".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://plain-context".to_string()),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::Mcp,
        })
        .await
        .expect("create")
        .value;

    backend
        .update_resource(UpdateResource {
            open_meta_add: None,
            resource: created.id,
            title: Some("after".to_string()),
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: None,
            goal: None,
            move_to: None,
            context_ref: None,
            act: ActContext::default(),
            origin: Surface::Mcp,
        })
        .await
        .expect("the ordinary slug-shaped-name path must keep working");
}
