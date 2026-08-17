#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// `temper resource show <slug> --without body --format json` returns the full
/// `show` view **minus the body**: the identity/home/attribution fields plus both the
/// `managed_meta` and `open_meta` tiers — everything except the reconstructed markdown.
///
/// This is `--meta-only`'s replacement, and it is the same read: one `GET /api/resources/{id}`
/// with the `GET /content` round-trip skipped.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_without_body_returns_the_view_minus_the_body(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("meta-cli", None)
        .await
        .expect("ctx create");

    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: "Show Meta Test".to_string(),
        origin_uri: "test://e2e/show-meta".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "showmeta0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: "# Show Meta\n\nBody here.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"temper-stage": "in-progress"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
        act: Default::default(),
        sources: Vec::new(),
    };

    let created = app.client.ingest().create(&payload).await.expect("ingest");
    let id = created.id.as_uuid().to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "show",
            id.as_str(),
            "--without",
            "body",
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
    assert!(stdout.get("id").is_some(), "missing id anchor: {stdout}");
    assert!(stdout.get("managed_meta").is_some(), "missing managed_meta");
    // The identity/display fields are included — a body-less `show` is the full view
    // minus one section, not a narrower projection.
    assert_eq!(
        stdout.get("title").and_then(Value::as_str),
        Some("Show Meta Test"),
        "title must be present: {stdout}"
    );
    assert_eq!(
        stdout.get("doc_type_name").and_then(Value::as_str),
        Some("task"),
        "doc_type_name must be present: {stdout}"
    );
    // Now that the title is present, the decorated `ref` is emitted too (parity with
    // the full `show`).
    assert!(stdout.get("ref").is_some(), "ref must be present: {stdout}");
    // The body is the one thing `--without body` withholds — and it is ABSENT, not `""`.
    assert!(stdout.get("content").is_none(), "should not include body");
    assert!(stdout.get("markdown").is_none(), "should not include body");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_without_body_with_fields_filters_response(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("meta-cli", None)
        .await
        .expect("ctx create");

    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: "Fields Filter Test".to_string(),
        origin_uri: "test://e2e/fields-filter".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "fieldsfilt0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"temper-stage": "backlog"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
        act: Default::default(),
        sources: Vec::new(),
    };
    let created = app.client.ingest().create(&payload).await.expect("ingest");
    let id = created.id.as_uuid().to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "show",
            id.as_str(),
            "--without",
            "body",
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
    assert!(stdout.get("id").is_some(), "anchor missing");
    assert!(stdout.get("managed_meta").is_some(), "managed_meta missing");
    assert!(
        stdout.get("open_meta").is_none(),
        "open_meta should be filtered"
    );
    // `managed_hash` no longer exists on the wire at all (§7-dissolved, field removed),
    // so this holds structurally rather than because `--fields` filtered it.
    assert!(stdout.get("managed_hash").is_none(), "hash must not exist");
}

/// Dotted path in --fields triggers a validation error mentioning "jq" and
/// the rejected path. The validation fires post-API-call (projection is applied
/// to the fetched meta), so the resource must exist to reach that code path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_without_body_with_dotted_path_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client
        .contexts()
        .create("meta-cli", None)
        .await
        .expect("ctx");

    // The dotted-path error fires after the API call (projection is applied
    // to the fetched meta), so the resource must exist.
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: "Dotted Path Test".to_string(),
        origin_uri: "test://e2e/dotted-path".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "dottedpath000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"temper-stage": "backlog"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
        act: Default::default(),
        sources: Vec::new(),
    };
    let created = app.client.ingest().create(&payload).await.expect("ingest");
    let id = created.id.as_uuid().to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "show",
            id.as_str(),
            "--without",
            "body",
            "--fields",
            "managed_meta.stage",
        ],
    )
    .await
    .expect("cli run");

    assert!(!output.status.success(), "expected non-zero exit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // In JSON mode (non-TTY piped stdout), the error rides stdout as a
    // structured ErrorPayload. The message should mention jq and echo the
    // rejected dotted path.
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout is a JSON ErrorPayload: {stdout:?}");
    let message = parsed["message"].as_str().expect("message field");
    assert!(
        message.contains("jq"),
        "message should mention jq: {message}"
    );
    assert!(
        message.contains("managed_meta.stage"),
        "message should echo the rejected path: {message}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_with_open_meta_returns_the_one_list_shape(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client
        .contexts()
        .create("meta-cli", None)
        .await
        .expect("ctx");

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
            idempotency_key: None,
            segmented: None,
            goal: None,
            title: format!("List Meta {slug}"),
            origin_uri: format!("test://e2e/{slug}"),
            context_ref: "@me/meta-cli".to_string(),
            home_cogmap_id: None,
            doc_type_name: "task".to_string(),
            content_hash: Some(hash.to_string()),
            // EMPTY body on purpose: client-ingested resources carry their prose in
            // `chunks_packed` (not `content`), so `content` arrives empty on the wire
            // and the resource's `body_hash` is the empty hash. A NON-empty `content`
            // would engage `create_resource`'s body-dedup, which then collapses these
            // two empty-bodied rows onto the same (empty) hash → one row. An empty
            // body skips dedup entirely, so both distinct rows persist (this is what
            // the stage-filter seed does too).
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-stage": "in-progress"})),
            open_meta: None,
            chunks_packed: Some(pack_chunks(&[]).unwrap()),
            act: Default::default(),
            sources: Vec::new(),
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
            "@me/meta-cli",
            "--with",
            "open-meta",
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
        assert!(row.get("id").is_some(), "row missing id anchor");
        assert!(
            row.get("managed_meta").is_some(),
            "row missing managed_meta"
        );
        // Rows are `ResourceView`s — the one shape — so they carry identity/display fields
        // and the decorated `ref`, not just the meta tiers.
        assert!(
            row.get("title").is_some(),
            "row missing title (should be a full detail row now): {row}"
        );
        assert!(
            row.get("doc_type_name").is_some(),
            "row missing doc_type_name: {row}"
        );
        assert!(row.get("ref").is_some(), "row missing decorated ref: {row}");
    }
    assert!(stdout.get("total").is_some(), "envelope missing total");
    assert!(stdout.get("facets").is_some(), "envelope missing facets");
}

/// `temper resource list --type task --context @me/meta-cli --fields origin_uri,managed_meta
/// --format json` (with no `--with`) should filter each `ResourceView` in the envelope rows
/// to include only the anchor field `id` plus the requested fields. Fields not in the selection
/// (`title`, `created`, `updated`, `body_hash`) must be absent.
///
/// The selection asks for `managed_meta`, not `stage`: `stage` was a hoisted column on the
/// retired `ResourceRow` and is not a top-level field of `ResourceView` — it lives under
/// `managed_meta` as `temper-stage`. `--fields` is a **top-level** projection, so naming a
/// field that no longer exists would filter to nothing and assert nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_default_with_fields_filters_response(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client
        .contexts()
        .create("meta-cli", None)
        .await
        .expect("ctx");

    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: "List Fields Test".to_string(),
        origin_uri: "test://e2e/list-fields-test".to_string(),
        context_ref: "@me/meta-cli".to_string(),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "listfields00000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"temper-stage": "in-progress"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).unwrap()),
        act: Default::default(),
        sources: Vec::new(),
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
            "@me/meta-cli",
            "--fields",
            "origin_uri,managed_meta",
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
        assert!(
            row.get("managed_meta").is_some(),
            "managed_meta missing in row: {row}"
        );
        assert_eq!(
            row["managed_meta"]["temper-stage"], "in-progress",
            "the workflow value went home to the managed tier, it did not go away: {row}"
        );
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
