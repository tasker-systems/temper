#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_health_check(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK");

    let body: Value = resp.json().await.expect("expected JSON body");
    assert_eq!(body["status"], "ok", "status field should be 'ok'");
}

/// The clause, asked of a *running service* rather than of a function.
///
/// `the-commit-serving-an-environment-is-knowable-without-inference` is about what an
/// environment will tell you when you ask it — so the witness has to ask over the wire.
/// The unit tests beside the handler establish the precondition (a build that was told a
/// commit reports that commit); this establishes that the answer is reachable: the route
/// is mounted, it is unauthenticated, and the serialized body carries the slot.
///
/// It bites against the pre-#584 state exactly. Before that PR `/api/health` returned
/// `{"status":"ok","version":"0.1.0"}` — 200, healthy, and silent about which commit was
/// serving it, which is the condition that let `main` and the running binary diverge for
/// forty minutes with every check green.
///
/// **This file is `#![cfg(feature = "test-db")]`, so it runs in the Integration job and
/// not the Unit one — and the Integration job deliberately leaves `VERCEL_GIT_COMMIT_SHA`
/// unset.** That is what keeps the `null` branch below live: with a commit always
/// present, "the key is there" would be satisfied by any build and would no longer
/// notice a `skip_serializing_if` dropping it. The match stays total anyway, so if that
/// job ever does supply a commit this strengthens rather than going quiet.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_running_service_answers_which_commit_it_is(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK");

    let body: Value = resp.json().await.expect("expected JSON body");
    let object = body.as_object().expect("the health body is a JSON object");

    assert!(
        object.contains_key("commit"),
        "a running service must answer which commit it is without anyone inferring it \
         from a merge, a branch name or a green check — and this body carries no `commit` \
         key at all, which is the pre-#584 shape: {body}"
    );

    let told = std::env::var("VERCEL_GIT_COMMIT_SHA")
        .ok()
        .map(|sha| sha.trim().to_string())
        .filter(|sha| !sha.is_empty());

    match (told, object["commit"].as_str()) {
        (Some(told), Some(reported)) => assert_eq!(
            reported, told,
            "the service must serve the commit its binary was built from"
        ),
        (Some(told), None) => panic!(
            "this build was told VERCEL_GIT_COMMIT_SHA={told}, so the running service must \
             report that commit rather than {}",
            object["commit"]
        ),
        (None, Some(reported)) => panic!(
            "this build was told no commit, yet the running service reports {reported:?} — \
             absence must be reported as absence, never as a plausible-looking value"
        ),
        (None, None) => assert!(
            object["commit"].is_null(),
            "an unrecorded commit is served as `null`, not as {}",
            object["commit"]
        ),
    }
}
