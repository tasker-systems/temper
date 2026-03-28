#[cfg(feature = "test-db")]
mod common;

/// Swagger UI does not serve content when disabled (ENABLE_SWAGGER not set).
/// Note: axum's auth middleware layer on the protected router applies to the
/// fallback handler, so unmatched paths return 401 rather than 404. The key
/// assertion is that swagger content is NOT served (no 200).
#[cfg(feature = "test-db")]
#[tokio::test]
async fn swagger_ui_not_served_when_disabled() {
    let app = common::setup_test_app().await;
    let resp = app
        .client
        .get(app.url("/api-docs/ui"))
        .send()
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        200,
        "Swagger UI should not be accessible when disabled"
    );
}

/// OpenAPI JSON does not serve content when disabled.
#[cfg(feature = "test-db")]
#[tokio::test]
async fn openapi_json_not_served_when_disabled() {
    let app = common::setup_test_app().await;
    let resp = app
        .client
        .get(app.url("/api-docs/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        200,
        "OpenAPI JSON should not be accessible when disabled"
    );
}
