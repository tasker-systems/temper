//! T7 — anchor-agnostic wayfind, driven at the **production caller's level** (spec §3.7).
//!
//! The task's acceptance criterion is explicit that this must be exercised "through `temper search`,
//! not only a direct `wayfind_scope_ids` call", and that matters here more than usual: the thing that
//! actually made `temper search --context @me/temper --wayfind` a hard failure was **not** the server.
//! It was a *client-side* mutual-exclusion guard in `build_search_params`, which rejected the flag
//! combination before any network round-trip. A test that posts JSON at `/api/search` would have gone
//! green while the real CLI still refused the command.
//!
//! So these drive the genuine chain — `temper_cli::actions::search::build_search_params` (the CLI's own
//! arg → `SearchParams` step) → `temper_client::search_with_params` → the real Axum app → Postgres —
//! and assert on what a user would actually get back.
#![cfg(feature = "test-db")]

mod common;

use temper_cli::actions::search::{build_search_params, CliSearchArgs};
use temper_core::types::ingest::IngestPayload;
use uuid::Uuid;

/// The CLI arg shape for a plain wayfind, with an optional `--context` scope. Mirrors what clap fills
/// in for `temper search <query> [--context <ref>] --wayfind`.
fn cli_args<'a>(query: &'a str, context: Option<&'a str>) -> CliSearchArgs<'a> {
    CliSearchArgs {
        query,
        embedding: None,
        context,
        cogmap: &[],
        wayfind: true,
        lens: None,
        regions: Some(10),
        doc_type: None,
        limit: Some(50),
        seed_ids: vec![],
        edge_types: vec![],
        depth: None,
        no_graph: true,
        seed_only: false,
    }
}

/// Ingest a context-homed resource — the "raw work" wayfind could not reach before T7.
async fn ingest_into_context(app: &common::E2eTestApp, title: &str, slug: &str, content: &str) {
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://wayfind-e2e/{slug}/{}", Uuid::new_v4()),
        context_ref: "@me/temper".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(content)),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");
}

/// `temper search --context @me/temper --wayfind` — the headline acceptance criterion.
///
/// The day before this migration this exact call was a hard error, and not even a server one: the CLI
/// rejected it locally with "--context, --cogmap, and --wayfind are mutually exclusive". Here it must
/// build, round-trip, and come back 200 with the context's own content.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn temper_search_context_wayfind_works(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    app.client
        .contexts()
        .create("temper", None)
        .await
        .expect("context create");

    ingest_into_context(
        &app,
        "zwayfinde2e raw work",
        "zwayfinde2e-raw",
        "zwayfinde2e the raw work homed in a context, which wayfind could not reach before T7.",
    )
    .await;

    // The CLI's own arg → SearchParams step. This is the guard that used to reject the command.
    let params = build_search_params(cli_args("zwayfinde2e", Some("@me/temper"))).expect(
        "`--context @me/temper --wayfind` must build: it was a client-side BadRequest before T7",
    );
    assert!(params.wayfind, "wayfind must survive onto the wire");
    assert_eq!(
        params.context_ref.as_deref(),
        Some("@me/temper"),
        "the context must ride along as the wayfind ANCHOR, not be dropped"
    );

    let resp = app
        .client
        .search()
        .search_with_params(&params)
        .await
        .expect("`temper search --context @me/temper --wayfind` must round-trip, not 400");

    let titles: Vec<&str> = resp.results.iter().map(|r| r.title.as_str()).collect();
    assert!(
        titles.iter().any(|t| t.contains("zwayfinde2e raw work")),
        "a context-scoped wayfind must return the context's own content — before T7 this content was \
         unreachable by wayfind BY CONSTRUCTION, and the `WAYFIND_UNREACHABLE` hint said so; got {titles:?}"
    );
}

/// An **unscoped** `temper search --wayfind` must also reach context-homed content now — the pool is
/// every visible anchor, not just the cogmaps. This is the half that makes the composition read work:
/// one wayfind, both the distilled idea and the raw work.
///
/// Guards the removed hint too: a `NoMatch` under wayfind must no longer tell agents that
/// context-homed content is "unreachable here regardless of phrasing". That guidance is now false, and
/// false guidance is worse than none — it teaches an agent to stop asking for the thing that works.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unscoped_wayfind_reaches_context_homed_content(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    app.client
        .contexts()
        .create("temper", None)
        .await
        .expect("context create");

    ingest_into_context(
        &app,
        "zwayfinde2e unscoped target",
        "zwayfinde2e-unscoped",
        "zwayfinde2e context-homed content that an unscoped wayfind must now pool in.",
    )
    .await;

    let params =
        build_search_params(cli_args("zwayfinde2e", None)).expect("plain --wayfind builds");
    let resp = app
        .client
        .search()
        .search_with_params(&params)
        .await
        .expect("unscoped wayfind must round-trip");

    let titles: Vec<&str> = resp.results.iter().map(|r| r.title.as_str()).collect();
    assert!(
        titles
            .iter()
            .any(|t| t.contains("zwayfinde2e unscoped target")),
        "an UNSCOPED wayfind must pool the principal's context regions, not only cogmaps; got {titles:?}"
    );

    if let Some(hint) = resp.diagnostics.as_ref().and_then(|d| d.hint.as_deref()) {
        assert!(
            !hint.contains("unreachable"),
            "the WAYFIND_UNREACHABLE guidance must be gone from the live diagnostics; got {hint:?}"
        );
    }
}
