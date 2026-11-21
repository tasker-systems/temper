#![cfg(feature = "test-db")]
//! A `temper resource create` re-issued with the same `--idempotency-key` returns the first
//! write's resource, never a second; different keys create two.
//!
//! The server claims `(owner, idempotency_key)` atomically with the create (issue #581); this
//! drives the real binary against the in-process API, so the key travels the CLI's own path. A
//! retry after a lost answer is exactly this: a second invocation carrying the first one's key.
//! That one invocation sends one key on every attempt, and that a network error names it, is
//! witnessed in `crates/temper-cli/tests/create_idempotency_key_test.rs`.

mod common;

use serde_json::Value;
use uuid::Uuid;

fn stdout_json(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "create failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON document ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

async fn create(app: &common::E2eTestApp, title: &str, key: &str) -> Value {
    let output = common::run_temper_cli(
        app,
        &[
            "--format",
            "json",
            "resource",
            "create",
            "--type",
            "session",
            "--title",
            title,
            "--context",
            "@me/idempotency-key-ctx",
            "--idempotency-key",
            key,
        ],
    )
    .await
    .expect("spawn the temper binary");
    stdout_json(&output)
}

async fn count_titled(pool: &sqlx::PgPool, title: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_resources WHERE title = $1 AND is_active")
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("count resources")
}

/// FAILS IF a create re-issued with its first key mints a second resource or answers with a
/// different resource than the first; or if two different keys are collapsed into one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_same_key_twice_is_one_resource_and_different_keys_are_two(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("idempotency-key-ctx", None)
        .await
        .expect("ctx create");

    let title = "Idempotency Key Replay Probe";
    let key = Uuid::now_v7().to_string();

    let first = create(&app, title, &key).await;
    let replay = create(&app, title, &key).await;
    assert_eq!(
        replay["id"], first["id"],
        "a replayed key answers with the first resource"
    );
    for field in [
        "ref",
        "title",
        "doc_type_name",
        "context_ref",
        "body_hash",
        "created",
    ] {
        assert_eq!(replay[field], first[field], "{field}: {first} vs {replay}");
    }
    assert_eq!(count_titled(&pool, title).await, 1, "one resource");

    let other = create(&app, title, &Uuid::now_v7().to_string()).await;
    assert_ne!(
        other["id"], first["id"],
        "a different key is a different create"
    );
    assert_eq!(count_titled(&pool, title).await, 2, "two resources");
}
