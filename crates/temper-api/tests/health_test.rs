#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;

#[tokio::test]
async fn test_health_check() {
    let app = common::setup_test_app().await;

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

#[tokio::test]
async fn test_openapi_json_endpoint() {
    let app = common::setup_test_app().await;

    let resp = app
        .client
        .get(app.url("/api-docs/openapi.json"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK");

    let body: Value = resp.json().await.expect("expected JSON body");
    assert_eq!(body["info"]["title"], "Temper Cloud API");
    assert!(body["paths"]["/api/resources"].is_object());
    assert!(body["components"]["securitySchemes"]["bearer_auth"].is_object());
}

#[tokio::test]
async fn test_swagger_ui_serves() {
    let app = common::setup_test_app().await;

    let resp = app
        .client
        .get(app.url("/api-docs/ui/"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK");

    let body = resp.text().await.expect("expected text body");
    assert!(
        body.contains("swagger-ui"),
        "should contain swagger-ui HTML"
    );
}
