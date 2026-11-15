#![cfg(feature = "test-db")]
//! The operator's join: a caller quotes the `x-request-id` it was refused with, and the `auth
//! error` line for that refusal carries the same id and the cause, at a `warn` log filter. At that
//! filter the info-level root span is not enabled for the log layer, so an id carried only on the
//! span would not be printed; the line must carry it itself. Its own test binary, because the
//! subscriber is process-global.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// FAILS IF: the `auth error` line for a pre-identity refusal does not carry the response's request
// id and the cause at a `warn` filter (the line moved back under the root span, or lost a field), or
// carries the caller's own `x-request-id`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_auth_error_line_carries_the_request_id_and_the_cause(pool: PgPool) {
    let capture = Capture::default();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(capture.clone())
                .with_filter(tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let app = common::setup_test_app(pool).await;
    let response = app
        .client
        .get(app.url("/api/profile"))
        .header("x-request-id", "caller-chosen")
        .send()
        .await
        .expect("request sent");
    assert_eq!(response.status().as_u16(), 401);
    let id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .expect("x-request-id")
        .to_string();

    let logged = String::from_utf8(capture.0.lock().unwrap().clone()).expect("utf-8 log");
    let line: serde_json::Value = logged
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|l| l["fields"]["message"] == "auth error")
        .unwrap_or_else(|| panic!("no auth error line at warn: {logged}"));
    assert_eq!(
        line["fields"]["request_id"],
        serde_json::json!(id),
        "{line}"
    );
    assert_eq!(line["fields"]["cause"], "missing_bearer", "{line}");
    assert!(
        !logged.contains("caller-chosen"),
        "the caller's id was logged: {logged}"
    );
}
