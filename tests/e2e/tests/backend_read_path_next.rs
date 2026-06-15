#![cfg(all(feature = "test-db", feature = "next-backend"))]

//! WS6 chunk 4b: under `flag=next`, the read endpoints answer from the `temper_next.*` substrate over
//! the REAL HTTP stack (axum handler → `select_backend` → `NextBackend` → `readback` → JSON), at the §9
//! invariant floor — the surface analogue of 4a's gate-wiring test, now returning real synthesized data.
//!
//! Proof shape (mutate-public-after-synthesis): seed + synthesize under legacy, then MUTATE the title in
//! `public` only. `temper_next` keeps the pre-mutation title. So a `next` read returning the
//! pre-mutation title proves the read came from `temper_next`, NOT `public` — a negative control that
//! sidesteps visibility subtleties (`NextBackend` reads are §9-unscoped by design).
//!
//! Local-only: no CI job enables `next-backend`. Run with
//! `cargo nextest run -p temper-e2e --features test-db,next-backend`.

mod common;

use reqwest::StatusCode;
use temper_api::backend::BackendSelection;

/// The metadata-only resource `common::clean_and_seed` inserts (`test://seed-resource`).
const SEED_RESOURCE_ID: &str = "00000000-0000-0000-0099-000000000001";

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn next_http_read_answers_from_temper_next_not_public(pool: sqlx::PgPool) {
    // 1. Legacy setup: base profile + the seeded metadata-only resource in `public`.
    let app = common::setup(pool).await;

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
