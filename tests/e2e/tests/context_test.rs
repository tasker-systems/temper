#![cfg(feature = "test-db")]

mod common;

/// Create a context, list all contexts, verify the new one appears.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_create_and_list(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let created = app
        .client
        .contexts()
        .create("e2e-context-list-test")
        .await
        .expect("context create failed");

    assert_eq!(created.name, "e2e-context-list-test");

    let contexts = app
        .client
        .contexts()
        .list()
        .await
        .expect("context list failed");

    assert!(
        contexts.iter().any(|c| c.id == created.id),
        "created context not found in list"
    );
}

/// Create a context, get it by ID, verify name matches.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_get_by_id(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let created = app
        .client
        .contexts()
        .create("e2e-context-get-by-id")
        .await
        .expect("context create failed");

    let fetched = app
        .client
        .contexts()
        .get(created.id.into())
        .await
        .expect("context get by id failed");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "e2e-context-get-by-id");
}

/// Create a context, try creating same name again, expect a Conflict error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_duplicate_name_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("e2e-context-duplicate")
        .await
        .expect("first context create failed");

    let result = app.client.contexts().create("e2e-context-duplicate").await;

    assert!(
        result.is_err(),
        "duplicate context name should return an error"
    );

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("conflict") || err_str.contains("Conflict") || err_str.contains("409"),
        "expected conflict error, got: {err_str}"
    );
}
