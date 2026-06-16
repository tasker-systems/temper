#![cfg(feature = "test-db")]

//! WS6 chunk 4a: with the backend-selection flag set to `next`, the
//! backend-constructed surfaces must fail with the NotImplemented guard (the
//! `next` backend does not exist until 4b) — proving the selector seam is
//! wired into the live HTTP stack, not just unit-tested in isolation
//! (`backend::selection` tests in temper-api).
//!
//! The A/B is on a single endpoint — `GET /api/resources/{id}` (show) routes
//! through `select_backend` before any lookup, so for a random id the flag
//! flips the response from `404 Not Found` (legacy) to `500` gated (next).
//!
//! Coverage map for the gate:
//! - `select_backend` (backend-constructed reads/writes — show/create/update/
//!   delete): proven end-to-end here via show.
//! - `require_legacy_backend` (edge/relationship path): next-arm unit-proven
//!   (temper-api `backend::selection::require_legacy_refuses_next`); Legacy
//!   wiring proven by the edge handler suite. Same `AppState.backend_selection`
//!   field and call shape.
//! - MCP surface: the tools call the same selectors over the same `AppState`
//!   field (Legacy path proven by the temper-mcp suite). A dedicated
//!   MCP-transport e2e test waits on an MCP driver in the harness (follow-up).
//!
//! NOT gated by 4a (deliberate): the service-direct read paths — `list`,
//! `search`, `get_meta` — bypass the `Backend` trait by design (CLAUDE.md:
//! reads are service-direct passthroughs), so they do NOT route through
//! `select_backend`. Under `next` they would still read legacy `public.*`.
//! Repointing these to the new substrate is **4b's** job (the §9 read homes),
//! not the gate's — recorded here so it isn't mistaken for a gap in 4a.

mod common;

use reqwest::StatusCode;

/// A backend-constructed read (`show`) routes through the `next` arm when the flag is flipped — proving
/// the seam is wired into the live HTTP stack. The observed status depends on whether `NextBackend` is
/// compiled in:
/// - WITHOUT `next-backend` (the default e2e build / CI): the `next` arm is gated and surfaces as a
///   `500` "not implemented" guard (the 4a behavior).
/// - WITH `next-backend` (4b): `NextBackend` is functional; a random id is not found in `temper_next`,
///   so the read returns `404` — the SAME as legacy, proving the next arm reaches a real lookup (not a
///   gate). The substantive "next answers from temper_next" proof is `backend_read_path_next.rs`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn api_show_routes_through_next_arm(pool: sqlx::PgPool) {
    // Flip the flag to `next` BEFORE the server starts — startup reads it once
    // (the cutover model: a flip takes effect on the next redeploy / spawn).
    sqlx::query("UPDATE kb_backend_selection SET backend = 'next' WHERE id = true")
        .execute(&pool)
        .await
        .expect("set backend selection to next");

    let app = common::setup(pool).await;

    let random_id = "00000000-0000-0000-0000-0000000000aa";
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{random_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    #[cfg(not(feature = "next-backend"))]
    {
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "without next-backend, the next arm surfaces as a server error (gated)"
        );
        let body = resp.text().await.expect("read error body");
        assert!(
            body.contains("not implemented"),
            "the error body should name the gate, got: {body}"
        );
    }
    #[cfg(feature = "next-backend")]
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "with next-backend, the functional next arm returns 404 for a random id (not in temper_next)"
    );
}

/// Control: under the seeded default (`legacy`), the same endpoint reaches the
/// real backend and returns `404` for a random id — proving the test above is
/// a genuine behavior flip, not a broken server, and that the gate is the only
/// thing standing between `404` and `500`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn api_show_reaches_backend_under_legacy_default(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let random_id = "00000000-0000-0000-0000-0000000000aa";
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{random_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "the default legacy flag must reach the backend (404 for a random id, not gated)"
    );
}
