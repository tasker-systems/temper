//! Integration test — the standing clock fires on the resource write path (Set 3, Phase B, Task 6).
//!
//! `DbBackend::tick_resource_standing` is wired immediately after `tick_region_clocks` at the resource
//! create and update sites. This test proves that wiring end-to-end: it drives a resource create
//! through the real backend command path (the same path that reaches the create-site tick) and asserts
//! the `kb_resource_standing` memo row appeared as a side effect — *without* any explicit refresh call.
//! That single side-effect assertion is what proves the clock is on the write path; the memo's
//! component parity (r_parent etc.) is already covered at the substrate level by Task 5's wrapper test
//! (`temper-substrate/tests/evidential_standing.rs`).
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write path's
/// `resolve_emitter` + visibility gate require). Mirrors `segmented_backend_test.rs`'s inlined
/// fixture — each test-target crate keeps its own copy so it has no cross-target test-harness
/// dependency.
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

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free, so the
/// create lands without touching the server-side embedder. Mirrors `segmented_backend_test.rs`.
fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

/// **The acceptance criterion: the standing clock fires on a resource CREATE.**
///
/// The create commits, then `tick_resource_standing` refreshes the finding's standing memo as a side
/// effect — with no explicit refresh call anywhere in this test. If the wiring were missing, no
/// `kb_resource_standing` row would exist for the new resource.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_resource_create_ticks_the_standing_memo(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "standing-clock@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let created = backend
        .create_resource(CreateResource {
            slug: "zz-standing-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "ZZ standing probe".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://standing-probe".to_string()),
            chunks_packed: Some(one_chunk_packed("first segment", "aa")),
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create resource")
        .value;

    let memo_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_standing WHERE finding_id = $1")
            .bind(created.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("query kb_resource_standing");

    assert_eq!(
        memo_rows, 1,
        "the create-site standing clock must have UPSERTed exactly one memo row for the new finding"
    );

    // A fresh create carries no reinforcement provenance, so r_parent (the breadth term, a count of
    // block_provenance rows) is expected to be 0 — the memo exists, it simply has nothing to count
    // yet. This asserts the row is materialized with the expected zero baseline, not that provenance
    // was seeded (that is Task 5's substrate-level concern).
    let r_parent: f64 =
        sqlx::query_scalar("SELECT r_parent FROM kb_resource_standing WHERE finding_id = $1")
            .bind(created.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("query r_parent");
    assert_eq!(
        r_parent, 0.0,
        "a create with no reinforcement provenance leaves r_parent at its zero baseline"
    );
}
