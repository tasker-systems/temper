#![cfg(feature = "test-db")]
//! The self-gated cron doors answer only on the deployment's own host.
//!
//! On Vercel, the eight routes in `routes::embed_internal` are reachable through the public
//! production alias and through the UI's `/api` proxy, as well as through the generated
//! deployment URL that Vercel Cron calls. `ApiConfig::cron_host` pins them to the last of those.
//! These tests drive every door through the real router: a request on any other host is a 404
//! before the bearer is read, and a request on the pinned host still meets the bearer.

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

async fn status(app: &common::TestApp, path: &str, host: &str, bearer: Option<&str>) -> StatusCode {
    let mut req = app
        .client
        .get(app.url(path))
        .header(reqwest::header::HOST, host);
    if let Some(b) = bearer {
        req = req.bearer_auth(b);
    }
    req.send().await.expect("request").status()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_cron_door_is_a_404_off_the_pinned_host(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |c| {
        c.cron_host = Some(PINNED.to_string());
        c.embed_dispatch_secret = Some(SECRET.to_string());
    })
    .await;

    for door in cron_doors() {
        for public in ["temper-cloud.vercel.app", "temperkb.io"] {
            assert_eq!(
                status(&app, door, public, Some(SECRET)).await,
                StatusCode::NOT_FOUND,
                "{door} on {public}, even holding the secret, must not exist"
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
