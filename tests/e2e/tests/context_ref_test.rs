#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Create with `context_ref = "@me/<slug>"` → succeeds (finds context by owner+slug).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_create_with_at_me_slug_succeeds(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Create a context; derive the slug from the DB (slug = sluggify(name)).
    let ctx = app
        .client
        .contexts()
        .create("e2e-context-ref-test", None)
        .await
        .expect("context create failed");

    // Look up the slug directly — ContextRow doesn't expose it.
    let slug: String = sqlx::query_scalar!("SELECT slug FROM kb_contexts WHERE id = $1", *ctx.id)
        .fetch_one(&pool)
        .await
        .expect("get context slug");

    let context_ref = format!("@me/{slug}");

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Context Ref Test — @me/slug form".to_string(),
        origin_uri: "test://e2e/context-ref-at-me-slug".to_string(),
        context_ref,
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "c0010001000100010001000100010001000100010001000100010001000100010001".to_string(),
        ),
        content: "# Context Ref Test\n\n@me/slug form.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest with @me/slug ref should succeed");

    assert_eq!(resource.title, "Context Ref Test — @me/slug form");
    assert!(resource.is_active);
}

/// Create with `context_ref = "temper"` (bare name, no @) → 400 BAD_REQUEST.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_create_with_bare_name_returns_400(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Should Be Rejected".to_string(),
        origin_uri: "test://e2e/context-ref-bare-name".to_string(),
        context_ref: "temper".to_string(), // bare name — no @ or + prefix
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: "# Rejected".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    // The client's `create` returns an error for non-2xx. Check the status code
    // by sending raw via reqwest so we can inspect it.
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .bearer_auth(&app.token)
        .json(&payload)
        .send()
        .await
        .expect("request should complete");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "bare context name should be rejected with 400"
    );
}

/// Create with bare UUID of a visible context → succeeds.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_create_with_uuid_context_ref_succeeds(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let ctx = app
        .client
        .contexts()
        .create("e2e-context-uuid-ref-test", None)
        .await
        .expect("context create failed");

    // Use the UUID form directly.
    let context_ref = ctx.id.to_string();

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Context Ref Test — UUID form".to_string(),
        origin_uri: "test://e2e/context-ref-uuid".to_string(),
        context_ref,
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "d0010001000100010001000100010001000100010001000100010001000100010001".to_string(),
        ),
        content: "# Context Ref Test\n\nUUID form.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest with UUID context ref should succeed");

    assert_eq!(resource.title, "Context Ref Test — UUID form");
    assert!(resource.is_active);
}
