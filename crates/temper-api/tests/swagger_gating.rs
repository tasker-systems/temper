#[cfg(feature = "test-db")]
mod common;

/// Swagger UI is disabled by default (ENABLE_SWAGGER not set).
#[cfg(feature = "test-db")]
#[tokio::test]
async fn swagger_ui_returns_404_when_disabled() {
    let app = common::setup_test_app().await;
    let resp = app
        .client
        .get(app.url("/api-docs/ui"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

/// OpenAPI JSON is also disabled.
#[cfg(feature = "test-db")]
#[tokio::test]
async fn openapi_json_returns_404_when_disabled() {
    let app = common::setup_test_app().await;
    let resp = app
        .client
        .get(app.url("/api-docs/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
