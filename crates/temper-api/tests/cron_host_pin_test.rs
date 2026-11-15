#![cfg(feature = "test-db")]
//! The cron doors answer only on the deployment's own host.
//!
//! On Vercel, the nine routes in `routes::embed_internal` are reachable through the public
//! production alias and through the UI's `/api` proxy, as well as through the generated
//! deployment URL that Vercel Cron calls. `ApiConfig::cron_host` pins them to the last of those.
//! These tests drive every door through the real router: a request on any other host is refused
//! before the bearer is read, with the uniform pre-identity refusal (the same bytes as a missing or
//! wrong bearer, so the pin tells a caller nothing the bearer does not), and a request on the
//! pinned host still meets the bearer.

mod common;

use reqwest::StatusCode;
use sqlx::PgPool;

/// Every route `routes::embed_internal` mounts, read from that file's source above its test module
/// rather than listed here, so a door added there is witnessed here without anyone remembering to.
/// That file's own unit test holds the same set equal to `vercel.json`'s crons.
fn cron_doors() -> Vec<&'static str> {
    let source = include_str!("../src/routes/embed_internal.rs");
    let routes = source
        .split("#[cfg(test)]")
        .next()
        .expect("source precedes tests");
    let doors: Vec<&'static str> = routes
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .flat_map(|line| line.split('"').skip(1).step_by(2))
        .filter(|literal| literal.starts_with("/api/"))
        .collect();
    assert!(!doors.is_empty(), "no routes parsed from embed_internal.rs");
    doors
}

const PINNED: &str = "temper-cloud-a1b2c3d4e-team.vercel.app";
const SECRET: &str = "cron-host-pin-test-secret-0123456789";

async fn answer(
    app: &common::TestApp,
    path: &str,
    host: &str,
    bearer: Option<&str>,
) -> (StatusCode, String) {
    let mut req = app
        .client
        .get(app.url(path))
        .header(reqwest::header::HOST, host);
    if let Some(b) = bearer {
        req = req.bearer_auth(b);
    }
    let response = req.send().await.expect("request");
    let status = response.status();
    (status, response.text().await.expect("body"))
}

async fn status(app: &common::TestApp, path: &str, host: &str, bearer: Option<&str>) -> StatusCode {
    answer(app, path, host, bearer).await.0
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_cron_door_is_the_uniform_401_off_the_pinned_host(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| {
        c.cron_host = Some(PINNED.to_string());
        c.embed_dispatch_secret = Some(SECRET.to_string());
    })
    .await;

    for door in cron_doors() {
        for public in ["temper-cloud.vercel.app", "temperkb.io"] {
            let (off_host, off_body) = answer(&app, door, public, Some(SECRET)).await;
            assert_eq!(
                off_host,
                StatusCode::UNAUTHORIZED,
                "{door} on {public}, even holding the secret, is refused"
            );
            let (_, no_bearer_body) = answer(&app, door, PINNED, None).await;
            assert_eq!(
                off_body, no_bearer_body,
                "{door}: the pin's refusal is byte-identical to the missing bearer's"
            );
        }
        // On the pinned host the bearer is still the second check.
        assert_eq!(
            status(&app, door, PINNED, None).await,
            StatusCode::UNAUTHORIZED,
            "{door} on the pinned host without the bearer"
        );
        assert_eq!(
            status(&app, door, PINNED, Some("not-the-secret")).await,
            StatusCode::UNAUTHORIZED,
            "{door} on the pinned host with the wrong bearer"
        );
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn without_a_cron_host_the_bearer_alone_gates(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| {
        c.embed_dispatch_secret = Some(SECRET.to_string());
    })
    .await;

    for door in cron_doors() {
        assert_eq!(
            status(&app, door, "temper.selfhosted.example", None).await,
            StatusCode::UNAUTHORIZED,
            "{door}: off Vercel no host is pinned, and the missing bearer is the refusal"
        );
    }
}

// FAILS IF: a cron door refuses the caller it exists for (the pinned host with the secret), so a
// gate that refused everything would pass the refusal tests above. `warm` is left out: admitted,
// it loads the ONNX embedder, which a test should not; its gate is the same tier layer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_pinned_host_with_the_secret_is_admitted(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| {
        c.cron_host = Some(PINNED.to_string());
        c.embed_dispatch_secret = Some(SECRET.to_string());
    })
    .await;
    for door in cron_doors().into_iter().filter(|d| *d != "/api/embed/warm") {
        let (code, body) = answer(&app, door, PINNED, Some(SECRET)).await;
        assert_ne!(
            code,
            StatusCode::UNAUTHORIZED,
            "{door} refused its caller: {body}"
        );
    }
}

// FAILS IF: HEAD or a non-preflight OPTIONS on a cron door off the pinned host answers anything
// but the refusal (a 405 with the path's `Allow`, or a 200 from a method fallback).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn head_and_options_off_the_pinned_host_are_refused(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| {
        c.cron_host = Some(PINNED.to_string());
        c.embed_dispatch_secret = Some(SECRET.to_string());
    })
    .await;
    for door in cron_doors() {
        for method in [reqwest::Method::HEAD, reqwest::Method::OPTIONS] {
            let response = app
                .client
                .request(method.clone(), app.url(door))
                .header(reqwest::header::HOST, "temperkb.io")
                .bearer_auth(SECRET)
                .send()
                .await
                .expect("request");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {door} off the pinned host"
            );
            assert!(
                response.headers().get("allow").is_none(),
                "{method} {door} carries Allow"
            );
        }
    }
}
