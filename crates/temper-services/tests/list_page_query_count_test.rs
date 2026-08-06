//! The list read path costs a bounded number of statements per PAGE, not one per row.
//!
//! `list_select` used to loop `native_resource_row(...)` once per page id — the same defect
//! `readback::hit_identities` had already fixed for search ("50 results meant 51 queries"), left
//! in place on list behind a doc comment that answered a different question ("only the page's ids
//! are reconstructed (no all-rows N+1)" — true about all rows, false about the page).
//!
//! **This is a measurement, not a structural assertion.** sqlx emits one `tracing` event on target
//! `sqlx::query` per statement it executes (`sqlx-core/src/logger.rs`, `QueryLogger::finish`), so a
//! counting layer over the process's subscriber counts statements the way the database sees them.
//!
//! It lives in its own test target on purpose: the counter is process-global, and a sibling test
//! executing queries concurrently in the same binary would be counted into it. One test per
//! process makes the count attributable under `cargo test` as well as under nextest.
#![cfg(feature = "test-db")]

use std::sync::atomic::{AtomicUsize, Ordering};

use sqlx::PgPool;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceListParams;

/// Statements sqlx has executed since the last [`reset_statement_count`].
static STATEMENTS: AtomicUsize = AtomicUsize::new(0);

/// Counts one per sqlx statement. Every `sqlx::query`-target event is exactly one executed
/// statement — the normal and the slow-statement arms both emit on that target, so neither escapes.
struct CountStatements;

impl<S: tracing::Subscriber> Layer<S> for CountStatements {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() == "sqlx::query" {
            STATEMENTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn install_counter() {
    // Ignore an already-installed subscriber: the assertion below reads the counter, and a counter
    // that never moved fails loudly rather than passing vacuously.
    let _ = tracing::subscriber::set_global_default(Registry::default().with(CountStatements));
}

fn reset_statement_count() {
    STATEMENTS.store(0, Ordering::SeqCst);
}

fn statement_count() -> usize {
    STATEMENTS.load(Ordering::SeqCst)
}

/// Seed a substrate profile + a profile-owned `temper` context. Mirrors the inlined fixture in
/// `open_meta_roundtrip_test.rs` / `segmented_backend_test.rs`.
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

/// How many resources the page holds. Large enough that an N+1 (`ROWS + 1` statements) cannot be
/// mistaken for the batched cost under any plausible amount of slack.
const ROWS: usize = 12;

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn list_page_is_one_batched_read_not_one_query_per_row(pool: PgPool) {
    install_counter();

    let (profile, context) = seed_profile_with_context(&pool, "list-n-plus-1@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    for i in 0..ROWS {
        backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: format!("zz-n1-{i:02}"),
                doctype: "task".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: format!("zz-n1-{i:02}"),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                goal: None,
                origin_uri: Some(format!("test://zz-n1-{i:02}")),
                chunks_packed: None,
                content_hash: None,
                act: ActContext::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create");
    }

    reset_statement_count();
    let response = substrate_read::list_select(
        &pool,
        ProfileId::from(profile),
        ResourceListParams::default(),
    )
    .await
    .expect("list");
    let statements = statement_count();

    // The page also carries whatever the migration chain seeds and makes publicly visible (the L0
    // kernel's telos resource, measured: one row), so the page is at least ROWS — never exactly it.
    let page_rows = response.rows.len();
    assert!(
        page_rows >= ROWS,
        "precondition: the page holds every seeded row, so an N+1 would be visible; got {page_rows}"
    );
    assert!(
        statements > 0,
        "precondition: the counting layer is installed and counting — a zero here means the \
         measurement, not the read, is broken"
    );
    assert!(
        statements <= 3,
        "a {page_rows}-row page must cost a bounded number of statements (the page query plus the \
         batched identity readback), not one per row. Measured {statements}; an N+1 costs \
         {} or more",
        page_rows + 1
    );
}
