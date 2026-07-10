#![cfg(feature = "test-db")]

//! Wire-level proof that `X-Temper-Surface` reaches the event ledger — the assertion
//! `sdk_emitter_entity_e2e.rs` deferred to P2.
//!
//! Before this work, `temper-cli`'s cloud backend constructed `Surface::CliCloud`, threaded it
//! through the command, and dropped it at the HTTP boundary: every cloud-mode CLI write was
//! attributed `<handle>@web`, and the `<handle>@cli` entity provisioned for every profile was
//! never resolved. These tests fail against that world.
//!
//! Three vectors are exercised:
//! - the CLI's own client (`app.client`, built with `Surface::CliCloud`) lands `<handle>@cli`;
//! - a raw request bearing `X-Temper-Surface: sdk` lands `<handle>@sdk`;
//! - a request with no header still lands `<handle>@web`, and every untrusted claim degrades to
//!   `web` **without failing the request** — surface is provenance, never authorization.

mod common;

use sqlx::PgPool;
use temper_core::types::ingest::IngestPayload;
use temper_workflow::operations::{Surface, SURFACE_HEADER};

/// An empty-content ingest payload (no body → no embed, so this runs without `test-embed`) homed in
/// the profile's auto-provisioned `default` context. A typed `IngestPayload`, not inline JSON, so a
/// wire-shape drift is a compile error rather than a silent 400.
fn probe_payload(title: &str) -> IngestPayload {
    IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://surface/{title}"),
        context_ref: "@me/default".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        sources: Vec::new(),
        act: Default::default(),
        segmented: None,
    }
}

/// The emitter entity name on the most recent event for `handle`.
///
/// `kb_events.id` is UUIDv7, so `ORDER BY id DESC` is newest-first without needing to know which
/// anchor column a resource-create event populates.
async fn latest_emitter_for(pool: &PgPool, handle: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT e.name FROM kb_events ev \
         JOIN kb_entities e ON e.id = ev.emitter_entity_id \
         JOIN kb_profiles p ON p.id = e.profile_id \
         WHERE p.handle = $1 \
         ORDER BY ev.id DESC LIMIT 1",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("an event exists for this profile")
}

/// The handle of the only non-system profile — the auto-provisioned e2e principal. UUIDv7 ids are
/// time-sortable, so the runtime-created principal sorts after the migration-seeded `system`.
async fn handle_of_only_profile(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>("SELECT handle FROM kb_profiles ORDER BY id DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a profile exists")
}

/// POST `/api/ingest` via the raw reqwest client with an explicit (or absent) surface header, and
/// return the emitter name the write was attributed to. This is the honest way to test the sdk and
/// deny directions: no typed client can construct an `mcp` (or garbage) claim.
async fn create_via_reqwest(
    app: &common::E2eTestApp,
    title: &str,
    surface_header: Option<&str>,
) -> String {
    let mut req = app
        .reqwest_client
        .post(format!("{}/api/ingest", app.base_url()))
        .bearer_auth(&app.token)
        .json(&probe_payload(title));

    if let Some(value) = surface_header {
        req = req.header(SURFACE_HEADER, value);
    }

    let resp = req.send().await.expect("ingest request");
    assert!(
        resp.status().is_success(),
        "ingest failed ({}): {}",
        resp.status(),
        resp.text().await.unwrap_or_default(),
    );

    let handle = handle_of_only_profile(&app.pool).await;
    latest_emitter_for(&app.pool, &handle).await
}

/// The bug this task fixes: `temper-cli` in cloud mode now lands `<handle>@cli`, not `@web`.
/// `app.client` is built through `build_client_from` with `Surface::CliCloud` — it is the CLI's own
/// client, not a hand-rolled imitation.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_client_write_lands_at_cli_emitter(pool: PgPool) {
    let app = common::setup(pool).await;
    // The first authenticated request auto-provisions the profile, its per-surface emitter
    // entities, and its `default` context (all via direct inserts — no ledger event), so the
    // resource create below is the newest event.
    app.client
        .profile()
        .get()
        .await
        .expect("provision profile on first authenticated request");

    app.client
        .ingest()
        .create(&probe_payload("cli surface probe"))
        .await
        .expect("create via the CLI's own client");

    let handle = handle_of_only_profile(&app.pool).await;
    assert_eq!(
        latest_emitter_for(&app.pool, &handle).await,
        format!("{handle}@{}", Surface::CliCloud.marker()),
    );
}

/// An `X-Temper-Surface: sdk` request — the future `temper-rb` path — lands `<handle>@sdk`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sdk_header_lands_at_sdk_emitter(pool: PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("provision profile on first authenticated request");
    let handle = handle_of_only_profile(&app.pool).await;

    let emitter = create_via_reqwest(&app, "sdk surface probe", Some("sdk")).await;
    assert_eq!(emitter, format!("{handle}@sdk"));
}

/// A browser sends no such header. It still lands `@web` — this is the no-regression case.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn absent_header_lands_at_web_emitter(pool: PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("provision profile on first authenticated request");
    let handle = handle_of_only_profile(&app.pool).await;

    let emitter = create_via_reqwest(&app, "web surface probe", None).await;
    assert_eq!(emitter, format!("{handle}@web"));
}

/// The deny direction. `mcp` is untrusted by construction (`temper-mcp` never crosses this
/// boundary), garbage is untrusted, empty is untrusted, the wrong case (`CLI`) is untrusted, and an
/// injection-shaped value is untrusted. All degrade to `web`, and — the load-bearing half — none of
/// them fails the request. Surface is provenance, never authorization.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn untrusted_headers_degrade_to_web_without_failing(pool: PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("provision profile on first authenticated request");
    let handle = handle_of_only_profile(&app.pool).await;

    for claimed in ["mcp", "not-a-surface", "", "CLI", "sdk; drop table"] {
        let emitter =
            create_via_reqwest(&app, &format!("untrusted [{claimed}]"), Some(claimed)).await;
        assert_eq!(
            emitter,
            format!("{handle}@web"),
            "claimed surface {claimed:?} should have degraded to web",
        );
    }
}
