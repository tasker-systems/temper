//! Asking a list page for the `open-meta` SECTION costs a bounded number of statements per PAGE,
//! not two per row.
//!
//! The sibling target `list_page_query_count_test` measures the default page — identity only. This
//! one measures the page the **MCP `list_resources` tool** actually asks for, which carries the open
//! tier on every row (`sections=open-meta`). Filling that section used to run `readback::meta` once
//! per view, and `meta` is itself two statements (its own `ensure_visible`, then the property read),
//! so a 12-row page cost `2 + 2×12`. The same N+1 `hit_identities` had already ended for the
//! identity half, surviving one section over — which is exactly where the MCP surface lived.
//!
//! **This is a measurement, not a structural assertion.** sqlx emits one `tracing` event on target
//! `sqlx::query` per statement it executes (`sqlx-core/src/logger.rs`, `QueryLogger::finish`), so a
//! counting layer over the process's subscriber counts statements the way the database sees them.
//!
//! It lives in its own test target on purpose, for the same reason its sibling does: the counter is
//! process-global, and a concurrent test in the same binary would be counted into it.
#![cfg(feature = "test-db")]

use std::sync::atomic::{AtomicUsize, Ordering};

use sqlx::PgPool;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::resource_view::ResourceSection;
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
/// `list_page_query_count_test.rs`.
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

/// How many resources the page holds. Large enough that an N+1 cannot be mistaken for the batched
/// cost under any plausible amount of slack.
const ROWS: usize = 12;

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_meta_section_is_one_batched_read_not_two_queries_per_row(pool: PgPool) {
    install_counter();

    let (profile, context) =
        seed_profile_with_context(&pool, "open-meta-n-plus-1@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    for i in 0..ROWS {
        backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: format!("zz-om-{i:02}"),
                doctype: "task".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: format!("zz-om-{i:02}"),
                body: None,
                managed_meta: ManagedMeta::default(),
                // A real open tier, so the fill has something to return and an empty-map shortcut
                // could not make this pass vacuously.
                open_meta: Some(serde_json::json!({ "tags": [format!("t{i:02}")] })),
                goal: None,
                origin_uri: Some(format!("test://zz-om-{i:02}")),
                chunks_packed: None,
                content_hash: None,
                act: ActContext::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create");
    }

    let params = ResourceListParams {
        sections: Some(ResourceSection::OpenMeta.to_string()),
        ..ResourceListParams::default()
    };

    reset_statement_count();
    let response = substrate_read::list_select(&pool, ProfileId::from(profile), params)
        .await
        .expect("list");
    let statements = statement_count();

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
    // The section was genuinely filled, so this is not measuring a page that skipped the work.
    let filled = response
        .rows
        .iter()
        .filter(|row| row.open_meta.is_some())
        .count();
    assert_eq!(
        filled, page_rows,
        "precondition: every row carries the requested open tier"
    );
    assert!(
        response
            .rows
            .iter()
            .any(|row| row.open_meta.as_ref().is_some_and(|v| v["tags"].is_array())),
        "precondition: at least one row's open tier carries the seeded value, not just an empty map"
    );

    assert!(
        statements <= 4,
        "a {page_rows}-row page asking for `open-meta` must cost the page query plus the batched \
         identity readback plus ONE batched property read. Measured {statements}; the per-view fill \
         this replaced cost {} or more",
        2 + 2 * page_rows
    );
}
