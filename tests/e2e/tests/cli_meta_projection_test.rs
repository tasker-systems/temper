#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// `temper resource show <slug> --meta-only --format json` returns
/// the ResourceMetaResponse shape (resource_id + managed_meta + ...).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_meta_only_returns_meta_response_shape(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("meta-cli")
        .await
        .expect("ctx create");

    let payload = IngestPayload {
        title: "Show Meta Test".to_string(),
        origin_uri: "test://e2e/show-meta".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "showmeta0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "show-meta-test".to_string(),
        content: "# Show Meta\n\nBody here.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"stage": "in-progress"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
    };

    let created = app.client.ingest().create(&payload).await.expect("ingest");
    let id = created.id.as_uuid().to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "show",
            id.as_str(),
            "--meta-only",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(
        output.status.success(),
        "cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    assert!(
        stdout.get("resource_id").is_some(),
        "missing resource_id: {stdout}"
    );
    assert!(stdout.get("managed_meta").is_some(), "missing managed_meta");
    // Confirm we DON'T have the body or row fields
    assert!(stdout.get("content").is_none(), "should not include body");
    assert!(
        stdout.get("title").is_none(),
        "should not include row title"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_meta_only_with_fields_filters_response(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("meta-cli")
        .await
        .expect("ctx create");

    let payload = IngestPayload {
        title: "Fields Filter Test".to_string(),
        origin_uri: "test://e2e/fields-filter".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "fieldsfilt0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "fields-filter-test".to_string(),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"stage": "backlog"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
    };
    let created = app.client.ingest().create(&payload).await.expect("ingest");
    let id = created.id.as_uuid().to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "show",
            id.as_str(),
            "--meta-only",
            "--fields",
            "managed_meta",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(
        output.status.success(),
        "cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    assert!(stdout.get("resource_id").is_some(), "anchor missing");
    assert!(stdout.get("managed_meta").is_some(), "managed_meta missing");
    assert!(
        stdout.get("open_meta").is_none(),
        "open_meta should be filtered"
    );
    assert!(
        stdout.get("managed_hash").is_none(),
        "hash should be filtered"
    );
}

/// Dotted path in --fields triggers a validation error mentioning "jq" and
/// the rejected path. The validation fires post-API-call (projection is applied
/// to the fetched meta), so the resource must exist to reach that code path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_meta_only_with_dotted_path_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client.contexts().create("meta-cli").await.expect("ctx");

    // The dotted-path error fires after the API call (projection is applied
    // to the fetched meta), so the resource must exist.
    let payload = IngestPayload {
        title: "Dotted Path Test".to_string(),
        origin_uri: "test://e2e/dotted-path".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "dottedpath000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "dotted-path-test".to_string(),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"stage": "backlog"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
    };
    let created = app.client.ingest().create(&payload).await.expect("ingest");
    let id = created.id.as_uuid().to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "show",
            id.as_str(),
            "--meta-only",
            "--fields",
            "managed_meta.stage",
        ],
    )
    .await
    .expect("cli run");

    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jq"), "stderr should mention jq: {stderr}");
    assert!(
        stderr.contains("managed_meta.stage"),
        "stderr should echo the rejected path: {stderr}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_meta_only_returns_meta_list_response_shape(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client.contexts().create("meta-cli").await.expect("ctx");

    // Ingest two task resources
    for (slug, hash) in &[
        (
            "list-meta-a",
            "lista0000000000000000000000000000000000000000000000000000000000",
        ),
        (
            "list-meta-b",
            "listb0000000000000000000000000000000000000000000000000000000000",
        ),
    ] {
        let payload = IngestPayload {
            title: format!("List Meta {slug}"),
            origin_uri: format!("test://e2e/{slug}"),
            context_ref: "@me/meta-cli".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: Some(hash.to_string()),
            slug: slug.to_string(),
            // EMPTY body on purpose: client-ingested resources carry their prose in
            // `chunks_packed` (not `content`), so `content` arrives empty on the wire
            // and the resource's `body_hash` is the empty hash. A NON-empty `content`
            // would engage `create_resource`'s body-dedup, which then collapses these
            // two empty-bodied rows onto the same (empty) hash → one row. An empty
            // body skips dedup entirely, so both distinct rows persist (this is what
            // the stage-filter seed does too).
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"stage": "in-progress"})),
            open_meta: None,
            chunks_packed: Some(pack_chunks(&[]).unwrap()),
        };
        app.client.ingest().create(&payload).await.expect("ingest");
    }

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "list",
            "--type",
            "task",
            "--context",
            "meta-cli",
            "--meta-only",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(
        output.status.success(),
        "cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    let rows = stdout
        .get("rows")
        .expect("envelope.rows")
        .as_array()
        .expect("array");
    assert!(rows.len() >= 2, "expected at least 2 rows: {stdout}");
    for row in rows {
        assert!(row.get("resource_id").is_some(), "row missing resource_id");
        assert!(
            row.get("managed_meta").is_some(),
            "row missing managed_meta"
        );
    }
    assert!(stdout.get("total").is_some(), "envelope missing total");
    assert!(stdout.get("facets").is_some(), "envelope missing facets");
}

/// `temper resource list --type task --context meta-cli --fields origin_uri,stage --format json`
/// (without --meta-only) should filter each ResourceRow in the envelope rows to
/// include only the anchor field `id` plus the requested fields. Fields not in
/// the selection (`title`, `created`, `updated`, `body_hash`) must be absent.
/// Note: `slug` was removed from ResourceRow in the native-shape drop (WS6 Task 2).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_default_with_fields_filters_response(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client.contexts().create("meta-cli").await.expect("ctx");

    let payload = IngestPayload {
        title: "List Fields Test".to_string(),
        origin_uri: "test://e2e/list-fields-test".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "listfields00000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "list-fields-test".to_string(),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"stage": "in-progress"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
    };
    app.client.ingest().create(&payload).await.expect("ingest");

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "list",
            "--type",
            "task",
            "--context",
            "meta-cli",
            "--fields",
            "origin_uri,stage",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(
        output.status.success(),
        "cli failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    let rows = stdout
        .get("rows")
        .expect("envelope.rows")
        .as_array()
        .expect("array");
    assert!(!rows.is_empty(), "expected at least one row: {stdout}");
    for row in rows {
        // Anchor field is always preserved
        assert!(row.get("id").is_some(), "anchor `id` missing in row: {row}");
        // Requested fields present
        assert!(
            row.get("origin_uri").is_some(),
            "origin_uri missing in row: {row}"
        );
        assert!(row.get("stage").is_some(), "stage missing in row: {row}");
        // Fields NOT in the selection must be absent
        assert!(
            row.get("title").is_none(),
            "title should be filtered out: {row}"
        );
        assert!(
            row.get("created").is_none(),
            "created should be filtered out: {row}"
        );
        assert!(
            row.get("updated").is_none(),
            "updated should be filtered out: {row}"
        );
        assert!(
            row.get("body_hash").is_none(),
            "body_hash should be filtered out: {row}"
        );
    }
    assert!(stdout.get("total").is_some(), "envelope missing total");
    assert!(stdout.get("facets").is_some(), "envelope missing facets");
}
