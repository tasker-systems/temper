#![cfg(all(feature = "test-db", feature = "next-backend"))]

//! WS6 chunk 4b: under `flag=next`, the read endpoints answer from the `temper_next.*` substrate over
//! the REAL HTTP stack (axum handler → `select_backend` → `NextBackend` → `readback` → JSON), at the §9
//! invariant floor — the surface analogue of 4a's gate-wiring test, now returning real synthesized data.
//!
//! Proof shape (mutate-public-after-synthesis): seed + synthesize under legacy, then MUTATE the title in
//! `public` only. `temper_next` keeps the pre-mutation title. So a `next` read returning the
//! pre-mutation title proves the read came from `temper_next`, NOT `public`.
//!
//! Reads are visibility-SCOPED to the caller's profile (WS2): the readbacks gate through
//! `temper_next.resources_visible_to`. The seed is owned by `SYSTEM_PROFILE_ID`, so the test binds the
//! authenticated principal (`e2e-test-user`) to that profile — the reader owns what it reads — and the
//! scoped read returns 200. (Pre-WS2 these reads were unscoped, so this binding was unnecessary.)
//!
//! Local-only: no CI job enables `next-backend`. Run with
//! `cargo nextest run -p temper-e2e --features test-db,next-backend`.

mod common;

use reqwest::StatusCode;
use temper_api::backend::{read_selector, BackendSelection, NextBackend};
use temper_core::error::TemperError;
use temper_core::operations::{Backend, ShowResource, Surface};
use temper_core::types::ids::{ProfileId, ResourceId};

/// The metadata-only resource `common::clean_and_seed` inserts (`test://seed-resource`).
const SEED_RESOURCE_ID: &str = "00000000-0000-0000-0099-000000000001";

/// A principal that neither owns/originated nor holds a READ grant on the seed — so the seed is not
/// visible to it under `resources_visible_to`. A phantom production id is a valid non-owner: it matches
/// no home row and no grant. (Same shape as the write-path gate test's intruder.)
const INTRUDER_PROFILE_ID: &str = "00000000-0000-0000-00cc-0000000000ff";

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn next_http_read_answers_from_temper_next_not_public(pool: sqlx::PgPool) {
    // 1. Legacy setup: base profile + the seeded metadata-only resource in `public`.
    let app = common::setup(pool).await;

    // Bind the authenticated principal (`e2e-test-user`) to the seed's owner profile (SYSTEM) so the
    // WS2 visibility-scoped read is authorized — resolve_from_claims resolves by (provider, subject)
    // first, so this pre-bound link makes the HTTP caller BE the owner. (auth_provider = the test
    // server's configured provider name, "test-provider".)
    sqlx::query(
        "INSERT INTO kb_profile_auth_links \
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at) \
         VALUES (gen_random_uuid(), $1::uuid, 'test-provider', 'e2e-test-user', \
                 'e2e@test.example.com', true, now()) \
         ON CONFLICT DO NOTHING",
    )
    .bind(common::SYSTEM_PROFILE_ID)
    .execute(&app.pool)
    .await
    .expect("bind principal to seed owner");

    // 2. Add a manifest row so synthesis (which inner-joins `kb_resource_manifests`) carries the
    //    resource, with the workflow managed-meta that becomes temper_next properties.
    sqlx::query(
        "INSERT INTO kb_resource_manifests (resource_id, managed_meta, open_meta)
         VALUES ($1::uuid, $2::jsonb, '{}'::jsonb)
         ON CONFLICT (resource_id) DO UPDATE SET managed_meta = EXCLUDED.managed_meta",
    )
    .bind(SEED_RESOURCE_ID)
    .bind(serde_json::json!({
        "temper-type": "research",
        "temper-stage": "in-progress",
        "temper-mode": "build",
        "temper-effort": "M",
    }))
    .execute(&app.pool)
    .await
    .expect("insert manifest");

    // 3. Synthesize public -> temper_next.
    temper_next::synthesis::run(&app.pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");

    // 4. MUTATE `public` AFTER synthesis: change the title only in public. temper_next keeps the
    //    pre-mutation title — this is the negative control distinguishing next-reads-temper_next from
    //    next-reads-public.
    sqlx::query("UPDATE kb_resources SET title = 'MUTATED IN PUBLIC' WHERE id = $1::uuid")
        .bind(SEED_RESOURCE_ID)
        .execute(&app.pool)
        .await
        .expect("mutate public title");

    // Confirm the public mutation actually landed (so the negative control is real).
    let public_title: String =
        sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1::uuid")
            .bind(SEED_RESOURCE_ID)
            .fetch_one(&app.pool)
            .await
            .expect("read public title");
    assert_eq!(
        public_title, "MUTATED IN PUBLIC",
        "public title was mutated"
    );

    // 5. Serve the SAME pool under `Next` and read over HTTP.
    let next_addr = common::spawn_app_server(app.pool.clone(), BackendSelection::Next).await;
    let resp = app
        .reqwest_client
        .get(format!(
            "http://{next_addr}/api/resources/{SEED_RESOURCE_ID}"
        ))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("next GET");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "next read returns the synthesized resource (200)"
    );
    let body: serde_json::Value = resp.json().await.expect("next json");

    // The crux: next returns the temper_next (pre-mutation) title, NOT the mutated public one.
    assert_eq!(
        body["title"], "Seed Research Doc",
        "next read answers from temper_next (pre-mutation title)"
    );
    assert_ne!(
        body["title"], "MUTATED IN PUBLIC",
        "next read did NOT fall through to mutated public"
    );

    // The §9 invariant fields are reconstructed correctly over the HTTP stack.
    assert_eq!(body["origin_uri"], "test://seed-resource", "origin_uri");
    assert_eq!(body["context_name"], "temper", "context_name reconstructed");
    assert_eq!(
        body["doc_type_name"], "research",
        "doc_type_name reconstructed"
    );
    assert_eq!(body["stage"], "in-progress", "stage property");
    assert_eq!(body["mode"], "build", "mode property");
    assert_eq!(body["effort"], "M", "effort property");
}

// ── deny-status split: not-visible → 404, genuine fault → 500 ────────────────────
//
// Both single-resource reads (show/content/meta) flow through `readback`'s typed `ReadbackError`, which
// the surface maps so that a not-visible deny is a leak-safe 404 (never 403 — that confirms existence)
// while a genuine reconstruction fault stays 500. Pre-fix the surface collapsed every post-resolve error
// to NotFound, which masked real faults as 404. These two tests pin both arms apart, exercised
// backend-level (`NextBackend::show_resource`) like the write-path gate tests.

/// Manifest the seed resource (with workflow managed-meta) and synthesize public → temper_next. Returns
/// the synthesized (re-minted) resource id, resolved by the verbatim-carried `origin_uri`.
async fn seed_and_synthesize(pool: &sqlx::PgPool) -> uuid::Uuid {
    sqlx::query(
        "INSERT INTO kb_resource_manifests (resource_id, managed_meta, open_meta) \
         VALUES ($1::uuid, $2::jsonb, '{}'::jsonb) \
         ON CONFLICT (resource_id) DO UPDATE SET managed_meta = EXCLUDED.managed_meta",
    )
    .bind(SEED_RESOURCE_ID)
    .bind(serde_json::json!({ "temper-type": "research" }))
    .execute(pool)
    .await
    .expect("insert manifest");
    temper_next::synthesis::run(pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");
    sqlx::query_scalar("SELECT id FROM temper_next.kb_resources WHERE origin_uri = $1")
        .bind("test://seed-resource")
        .fetch_one(pool)
        .await
        .expect("synthesized resource id")
}

fn show_seed() -> ShowResource {
    ShowResource {
        resource: ResourceId::from(uuid::Uuid::parse_str(SEED_RESOURCE_ID).unwrap()),
        origin: Surface::CliCloud,
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn next_read_not_visible_returns_not_found(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    seed_and_synthesize(&app.pool).await;

    let owner = ProfileId::from(uuid::Uuid::parse_str(common::SYSTEM_PROFILE_ID).unwrap());
    let intruder = ProfileId::from(uuid::Uuid::parse_str(INTRUDER_PROFILE_ID).unwrap());

    // Positive control: the owner sees its own resource (the deny is not blanket).
    NextBackend::new(app.pool.clone(), owner)
        .show_resource(show_seed())
        .await
        .expect("owner read admitted");

    // Not-visible → NotFound (404): the leak-safe deny. NEVER Forbidden (403 confirms existence).
    let err = NextBackend::new(app.pool.clone(), intruder)
        .show_resource(show_seed())
        .await
        .expect_err("non-owner read must be denied");
    assert!(
        matches!(err, TemperError::NotFound(_)),
        "not-visible read must be NotFound (404), got {err:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn next_read_genuine_fault_returns_api_error(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let new_id = seed_and_synthesize(&app.pool).await;
    let owner = ProfileId::from(uuid::Uuid::parse_str(common::SYSTEM_PROFILE_ID).unwrap());

    // Positive control: the (visible) owner reads it cleanly before any fault injection.
    NextBackend::new(app.pool.clone(), owner)
        .show_resource(show_seed())
        .await
        .expect("owner read admitted before fault injection");

    // Inject a genuine reconstruction fault on the SAME visible resource: drop the `doc_type` property
    // every resource pass stamps. The resource stays visible (`ensure_visible` still passes), but
    // `resource_row`'s inner JOIN on the doc_type property now finds no row → a real DB fault, distinct
    // from a visibility deny.
    sqlx::query(
        "DELETE FROM temper_next.kb_properties \
         WHERE owner_table='kb_resources' AND owner_id=$1::uuid AND property_key='doc_type'",
    )
    .bind(new_id)
    .execute(&app.pool)
    .await
    .expect("delete doc_type property");

    // The fault is a system failure → Api (500), NOT NotFound (404). This is the deny-status split: the
    // resource is still visible, so collapsing this to 404 would mask a real fault.
    let err = NextBackend::new(app.pool.clone(), owner)
        .show_resource(show_seed())
        .await
        .expect_err("a genuine reconstruction fault must surface");
    assert!(
        matches!(err, TemperError::Api(_)),
        "genuine fault must be Api (500), not NotFound (404), got {err:?}"
    );
}

/// WS6 Spec B Task 4: the `read_selector::show_select` Next arm (which backs the rewritten MCP
/// `get_resource`) reconstructs the same §9-invariant-floor row as the Legacy arm. Drives the selector
/// directly over the synthesized fixture (full `temper_api::MIGRATOR`, so the WS2 visibility helpers
/// `resources_visible_to`/`profile_effective_teams` exist — unlike the temper-next artifact migrator).
/// Both arms take the production id; the Next arm bridges prod→new via `resolve_new_id` then
/// `reconstruct_resource_row`. Non-invariants (re-minted ids / §7-dissolved slug+hashes /
/// synthesis-collapsed timestamps) are NOT asserted equal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_select_next_matches_legacy_at_floor(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    seed_and_synthesize(&app.pool).await;

    let owner = uuid::Uuid::parse_str(common::SYSTEM_PROFILE_ID).unwrap();
    let prod_id = uuid::Uuid::parse_str(SEED_RESOURCE_ID).unwrap();

    let legacy = read_selector::show_select(BackendSelection::Legacy, &app.pool, owner, prod_id)
        .await
        .expect("legacy show");
    let next = read_selector::show_select(BackendSelection::Next, &app.pool, owner, prod_id)
        .await
        .expect("next show");

    assert_eq!(legacy.origin_uri, next.origin_uri, "origin_uri");
    assert_eq!(legacy.title, next.title, "title");
    assert_eq!(legacy.context_name, next.context_name, "context_name");
    assert_eq!(legacy.doc_type_name, next.doc_type_name, "doc_type_name");
    assert_eq!(legacy.is_active, next.is_active, "is_active");
}
