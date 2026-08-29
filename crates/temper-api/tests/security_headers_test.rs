#![cfg(feature = "test-db")]
//! The baseline response headers, and the one route that is allowed to relax them.
//!
//! `apply_base_layers` sets the baseline for both public surfaces; temper-mcp's
//! `transport_layers_test` witnesses it there, on the surface that was once assembled without the
//! shared stack. What that suite cannot reach is temper-api's HTML: the Slack callback renders a
//! page with an inline `<style>` block, so it sets its own content-security policy, and the value
//! of the whole `if_not_present` design is that this one route gets an exception while every other
//! route on either surface keeps the strict policy.
//!
//! A synthetic router proved the mechanism; this proves the real route uses it.
//!
//! `cargo nextest run -p temper-api --features test-db --test security_headers_test`

mod common;

use axum::http::StatusCode;
use sqlx::PgPool;

const STRICT_POLICY: &str = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'";
const CALLBACK_POLICY: &str =
    "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'";

/// A JSON route carries the strict policy — the one for a surface that loads nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_json_route_carries_the_strict_content_security_policy(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let res = app
        .client
        .get(app.url("/api/resources"))
        .send()
        .await
        .expect("request sends");

    assert_eq!(
        res.headers()
            .get("content-security-policy")
            .map(|v| v.to_str().expect("header is ascii")),
        Some(STRICT_POLICY),
        "every route that answers with JSON must carry the policy for a surface that renders \
         nothing; the HTML exception is one route wide and must not have leaked to the API"
    );
}

/// The Slack callback page relaxes exactly one directive, and keeps the rest of the baseline.
///
/// `?error=` is the earliest return in the handler — it renders the not-connected page before any
/// configuration is read or any row is touched — so this reaches the HTML without standing up a
/// Slack link flow. The page shape does not matter here; the headers on it do.
///
/// **Both halves are asserted, and the second is the point.** That the policy is relaxed shows the
/// exception works. That `x-content-type-options` survives shows the route took an exception to one
/// header rather than opting out of the baseline — which is the failure mode a test asserting only
/// the policy would wave through.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_slack_callback_page_relaxes_only_its_style_directive(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let res = app
        .client
        .get(app.url("/api/auth/slack/callback?error=denied"))
        .send()
        .await
        .expect("request sends");

    assert_eq!(res.status(), StatusCode::OK, "the callback always renders");

    assert_eq!(
        res.headers()
            .get("content-security-policy")
            .map(|v| v.to_str().expect("header is ascii")),
        Some(CALLBACK_POLICY),
        "the page carries an inline <style> block, so under the strict baseline it renders \
         unstyled; this is the exception that stops that, and it widens style-src and nothing else"
    );

    for (name, expected) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
    ] {
        assert_eq!(
            res.headers()
                .get(name)
                .map(|v| v.to_str().expect("header is ascii")),
            Some(expected),
            "{name} must survive the policy exception; relaxing one header is not opting out of \
             the baseline, and referrer-policy in particular is why this page is not a leak — its \
             URL carries the link state"
        );
    }
}

/// The Swagger explorer carries its own policy, and it is still origin-locked.
///
/// The explorer serves its own scripts, styles and images from this origin — all of which the
/// strict baseline forbids, so under it the page would load blank. Its exception is set on the
/// sub-router rather than on a response, which is a different mechanism from the Slack page's, and
/// therefore needs its own witness: a `.into()` conversion and a layer that composed wrongly would
/// leave the explorer either unreachable or silently broken, and only in a deployment with
/// `ENABLE_SWAGGER` set — the one place nobody is watching.
///
/// **What is asserted is the boundary, not the permissiveness.** `default-src 'self'` and
/// `frame-ancestors 'none'` are the load-bearing halves: whatever the explorer is allowed to load,
/// it is allowed to load it only from this origin, and the page cannot be framed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_swagger_explorer_carries_its_own_origin_locked_policy(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| c.enable_swagger = true).await;

    let res = app
        .client
        .get(app.url("/api-docs/openapi.json"))
        .send()
        .await
        .expect("request sends");

    assert_eq!(
        res.status(),
        StatusCode::OK,
        "the explorer's routes must still be reachable once wrapped in their own policy layer"
    );

    let policy = res
        .headers()
        .get("content-security-policy")
        .map(|v| v.to_str().expect("header is ascii"))
        .expect("the explorer's routes carry a policy");

    assert_ne!(
        policy, STRICT_POLICY,
        "under the strict baseline the explorer loads nothing and renders blank; this is the \
         exception that stops that"
    );
    assert!(
        policy.contains("default-src 'self'"),
        "the explorer may load its own bundle and nothing third-party; policy was {policy}"
    );
    assert!(
        policy.contains("frame-ancestors 'none'"),
        "the framing rule is not part of what the explorer needs relaxed; policy was {policy}"
    );
}
