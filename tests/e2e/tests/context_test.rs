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
        .create("e2e-context-list-test", None)
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
        .create("e2e-context-get-by-id", None)
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

/// Create a context, then create another with the SAME name. The substrate
/// is slug-keyed, not name-keyed: `context_service::create` calls
/// `next_unique_context_slug`, which auto-suffixes the generated slug on
/// collision so two contexts sharing a name coexist under distinct slugs
/// rather than 409ing (intentional D-task delta; see context_service.rs
/// `next_unique_context_slug` / `create` doc-comments). Both creates succeed,
/// keep the requested name, and mint distinct ids.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_duplicate_name_auto_suffixes_slug(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let first = app
        .client
        .contexts()
        .create("e2e-context-duplicate", None)
        .await
        .expect("first context create failed");

    let second = app
        .client
        .contexts()
        .create("e2e-context-duplicate", None)
        .await
        .expect("second create with duplicate name should succeed (slug auto-suffixed)");

    // Name is preserved on both; the substrate disambiguates by slug, not name.
    assert_eq!(first.name, "e2e-context-duplicate");
    assert_eq!(second.name, "e2e-context-duplicate");
    // Distinct rows: each create mints a fresh context id.
    assert_ne!(
        first.id, second.id,
        "duplicate-name creates must be distinct context rows"
    );
}
