#![cfg(feature = "test-db")]

//! `temper resource update`'s reference flags assert the edge they name, or refuse.
//!
//! Each flag used to write only its open-tier key and exit 0: nothing projects those keys into
//! the graph, so `graph traverse` and `resource connections` never saw the relation the flag's
//! success claimed. These witnesses drive the real binary against a live API and
//! read the edge back from `kb_edges`, so each fails against the silent drop.

mod common;

use serde_json::Value;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use uuid::Uuid;

/// Create a task in `@me/<ctx>` through the ingest door and return its id.
async fn create_task(app: &common::E2eTestApp, ctx: &str, title: &str) -> Uuid {
    let body = format!("# {title}\n\nBody.");
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://e2e/reference-flags/{title}"),
        context_ref: format!("@me/{ctx}"),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: None,
        content: body.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&common::chunked(&body, 0.1)).unwrap()),
        act: Default::default(),
        sources: Vec::new(),
    };
    let created = app.client.ingest().create(&payload).await.expect("ingest");
    *created.id.as_uuid()
}

async fn setup_context(app: &common::E2eTestApp, ctx: &str) {
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create(ctx, None)
        .await
        .expect("ctx create");
}

/// Every reference flag asserts its edge, with the structural triple the frontmatter relation
/// maps to, and the command's JSON names the edge it asserted.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn each_reference_flag_asserts_its_edge(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    setup_context(&app, "ref-flags").await;

    // (flag, label, edge_kind, polarity) — the triple is `EdgeType::legacy_mapping`'s.
    let cases = [
        ("--relates-to", "relates_to", "near", "forward"),
        ("--references", "references", "near", "forward"),
        ("--depends-on", "depends_on", "leads_to", "inverse"),
        ("--extends", "extends", "leads_to", "inverse"),
        ("--preceded-by", "preceded_by", "leads_to", "inverse"),
        ("--derived-from", "derived_from", "leads_to", "inverse"),
    ];

    for (flag, label, kind, polarity) in cases {
        let source = create_task(&app, "ref-flags", &format!("source {label}")).await;
        let target = create_task(&app, "ref-flags", &format!("target {label}")).await;
        let (source_s, target_s) = (source.to_string(), target.to_string());

        let output = common::run_temper_cli(
            &app,
            &[
                "resource",
                "update",
                source_s.as_str(),
                flag,
                target_s.as_str(),
                "--format",
                "json",
            ],
        )
        .await
        .expect("cli run");
        assert!(
            output.status.success(),
            "{flag}: cli failed: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );

        // The edge is in the graph, live, with the relation's triple.
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT edge_kind::text, polarity::text FROM kb_edges \
             WHERE source_id = $1 AND target_id = $2 AND label = $3 AND NOT is_folded",
        )
        .bind(source)
        .bind(target)
        .bind(label)
        .fetch_all(&app.pool)
        .await
        .expect("edge read");
        assert_eq!(
            rows,
            vec![(kind.to_string(), polarity.to_string())],
            "{flag} must assert exactly one live `{label}` edge {source} → {target}"
        );

        // The command says which edge it asserted.
        let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
        let asserted = stdout
            .get("edges_asserted")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{flag}: edges_asserted missing: {stdout}"));
        assert_eq!(asserted.len(), 1, "{flag}: {stdout}");
        assert_eq!(asserted[0]["label"], label, "{flag}: {stdout}");
        assert_eq!(asserted[0]["target"], target_s, "{flag}: {stdout}");

        // The open-tier key still records the declaration, as before.
        let meta = app.client.resources().get_meta(source).await.expect("meta");
        let open = serde_json::to_value(&meta.open_meta).expect("open tier");
        assert_eq!(
            open[label],
            serde_json::json!([target_s]),
            "{flag}: open tier {open}"
        );
    }
}

/// A value that is not a resource ref is refused before anything is written: no open-tier key,
/// no edge, and a non-zero exit naming the flag.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_reference_flag_refuses_a_value_that_is_not_a_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    setup_context(&app, "ref-flags-refuse").await;
    let source = create_task(&app, "ref-flags-refuse", "refuse source").await;
    let source_s = source.to_string();

    let output = common::run_temper_cli(
        &app,
        &[
            "resource",
            "update",
            source_s.as_str(),
            "--relates-to",
            "https://example.com/not-a-resource",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");
    assert!(
        !output.status.success(),
        "a non-ref --relates-to must not exit 0: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    // Under `--format json` the typed error is the JSON document on stdout.
    let error: Value = serde_json::from_slice(&output.stdout).expect("json error document");
    assert_eq!(error["code"], "bad-request", "a typed refusal: {error}");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|m| m.contains("--relates-to")),
        "the refusal names the flag: {error}"
    );

    let meta = app.client.resources().get_meta(source).await.expect("meta");
    let open = serde_json::to_value(&meta.open_meta).expect("open tier");
    assert!(
        open.get("relates_to").is_none(),
        "nothing was written: open tier {open}"
    );
    let edges: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges WHERE source_id = $1 AND label = 'relates_to'",
    )
    .bind(source)
    .fetch_one(&app.pool)
    .await
    .expect("edge count");
    assert_eq!(edges, 0, "no edge was asserted");
}
