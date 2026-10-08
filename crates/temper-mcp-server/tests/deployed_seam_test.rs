//! **The trap, pinned at the wire.** The deployed door's seam must reproduce, exactly, the
//! outgoing request the relay sent before the seam existed: the caller's bearer, the service
//! credential, the `mcp` carrier, the `mcp` surface, and no device id. Any slip changes
//! attribution (`<handle>@mcp` becomes `@web`) without failing a wire-shape test, so this suite
//! captures the request a relayed act actually sends and compares its identity headers to that
//! exact set.
//!
//! The optional edge-proxy marker (`TEMPER_EDGE_PROXY_SECRET`) is captured too: absent unless
//! configured, never equal to the service credential.
//!
//! The shell's configuration postures are pinned beside it: either variable unset (or a secret
//! that cannot ride a header) leaves the tool door dark with the deployment's own sentence, and
//! a request the edge never verified (no bearer) answers the tool layer's not-connected refusal.

use std::sync::{Arc, Mutex};

use temper_mcp::BlobDoor;
use temper_mcp_server::config::{deployed_relay, DeployedRelay, RELAY_REQUEST_TIMEOUT};
use temper_mcp_server::BearerToken;

const SECRET: &str = "a-service-secret-of-length";
const BEARER: &str = "the-callers-verified-bearer";

fn closed() -> BlobDoor {
    BlobDoor::Closed {
        refusal: "unused".to_string(),
    }
}

fn relay(base: Option<&str>, secret: Option<&str>) -> DeployedRelay {
    relay_marked(base, secret, None)
}

fn relay_marked(base: Option<&str>, secret: Option<&str>, marker: Option<&str>) -> DeployedRelay {
    let base = base.map(str::to_string);
    let secret = secret.map(str::to_string);
    let marker = marker.map(str::to_string);
    deployed_relay(&move |key: &str| match key {
        "TEMPER_API_BASE_URL" => base.clone(),
        "TEMPER_MCP_SERVICE_SECRET" => secret.clone(),
        "TEMPER_EDGE_PROXY_SECRET" => marker.clone(),
        _ => None,
    })
}

fn parts(bearer: Option<&str>) -> http::request::Parts {
    let mut builder = http::Request::builder();
    if let Some(b) = bearer {
        builder = builder.extension(BearerToken(b.to_string()));
    }
    builder.body(()).expect("parts build").into_parts().0
}

/// The identity headers the API reads, as one relayed request carried them.
type Captured = Arc<Mutex<Vec<(String, Option<String>)>>>;

const IDENTITY_HEADERS: [&str; 6] = [
    "authorization",
    "x-temper-service-credential",
    "x-temper-relayed-surface",
    "x-temper-surface",
    "x-temper-device-id",
    "x-temper-edge-proxy",
];

/// A listener that records the identity headers of every request and answers 500 (the relayed
/// act's outcome is not the subject; its credentials are).
async fn capture() -> (String, Captured) {
    let seen: Captured = Arc::default();
    let sink = seen.clone();
    let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let sink = sink.clone();
        async move {
            let mut s = sink.lock().unwrap();
            s.clear();
            for name in IDENTITY_HEADERS {
                s.push((
                    name.to_string(),
                    req.headers()
                        .get(name)
                        .map(|v| v.to_str().unwrap().to_string()),
                ));
            }
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the capture listener");
    let addr = listener.local_addr().expect("capture address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("capture serves") });
    (format!("http://{addr}"), seen)
}

/// FAILS IF: the deployed seam's request differs from today's in any identity header — the
/// bearer, the service credential, the `mcp` carrier, the `mcp` surface — or carries a device id.
#[tokio::test]
async fn the_deployed_seam_sends_exactly_the_relay_identity() {
    let (base, seen) = capture().await;
    let service = temper_mcp_server::tool_service(closed(), relay(Some(&base), Some(SECRET)));

    let client = service
        .relay_client(&parts(Some(BEARER)))
        .expect("a verified bearer builds a relay client");
    let _ = client.profile().get().await;

    let got = seen.lock().unwrap().clone();
    let want: Vec<(String, Option<String>)> = vec![
        ("authorization".into(), Some(format!("Bearer {BEARER}"))),
        ("x-temper-service-credential".into(), Some(SECRET.into())),
        ("x-temper-relayed-surface".into(), Some("mcp".into())),
        ("x-temper-surface".into(), Some("mcp".into())),
        ("x-temper-device-id".into(), None),
        ("x-temper-edge-proxy".into(), None),
    ];
    assert_eq!(got, want, "the deployed door's outgoing identity moved");
}

/// One relayed act's captured headers, under a given relay configuration.
async fn relayed_headers(marker: Option<&str>) -> Vec<(String, Option<String>)> {
    let (base, seen) = capture().await;
    let service =
        temper_mcp_server::tool_service(closed(), relay_marked(Some(&base), Some(SECRET), marker));
    let _ = service
        .relay_client(&parts(Some(BEARER)))
        .expect("relay client")
        .profile()
        .get()
        .await;
    let got = seen.lock().unwrap().clone();
    got
}

/// Configured, every relayed act carries the edge-proxy marker, so the API project's per-IP
/// rules can exempt the relay's hops — which all arrive from this function's few addresses.
/// FAILS IF: the marker is dropped, or any identity header moves beside it.
#[tokio::test]
async fn a_configured_edge_proxy_secret_marks_every_relayed_act() {
    let got = relayed_headers(Some("an-edge-proxy-marker")).await;
    let want: Vec<(String, Option<String>)> = vec![
        ("authorization".into(), Some(format!("Bearer {BEARER}"))),
        ("x-temper-service-credential".into(), Some(SECRET.into())),
        ("x-temper-relayed-surface".into(), Some("mcp".into())),
        ("x-temper-surface".into(), Some("mcp".into())),
        ("x-temper-device-id".into(), None),
        (
            "x-temper-edge-proxy".into(),
            Some("an-edge-proxy-marker".into()),
        ),
    ];
    assert_eq!(got, want);
}

/// A marker that cannot be a header value is not sent, and the relay still works: the marker
/// is never a reason to go dark. FAILS IF: the door darkens or a mangled marker rides the wire.
#[tokio::test]
async fn a_marker_that_cannot_be_a_header_value_is_not_sent() {
    let got = relayed_headers(Some("bad\u{1}marker")).await;
    assert!(got.contains(&("x-temper-edge-proxy".into(), None)));
    assert!(got.contains(&("x-temper-service-credential".into(), Some(SECRET.into()))));
}

/// The marker's value is copied into firewall configuration, so a marker equal to the service
/// secret is not sent — and the relay still works, because the marker is never a reason to go
/// dark. FAILS IF: the service credential rides the marker header, or the door darkens.
#[tokio::test]
async fn a_marker_equal_to_the_service_secret_is_not_sent() {
    let got = relayed_headers(Some(SECRET)).await;
    assert!(got.contains(&("x-temper-edge-proxy".into(), None)));
    assert!(got.contains(&("x-temper-service-credential".into(), Some(SECRET.into()))));
}

/// The secret is trimmed exactly as the API trims the same variable (`shared_secret`), so the
/// value presented is the value compared. FAILS IF: surrounding whitespace rides the header.
#[tokio::test]
async fn the_presented_secret_is_trimmed_as_the_api_trims_it() {
    let (base, seen) = capture().await;
    let padded = format!("  {SECRET}\n");
    let service = temper_mcp_server::tool_service(closed(), relay(Some(&base), Some(&padded)));
    let _ = service
        .relay_client(&parts(Some(BEARER)))
        .expect("relay client")
        .profile()
        .get()
        .await;
    let got = seen.lock().unwrap().clone();
    assert!(got.contains(&("x-temper-service-credential".into(), Some(SECRET.into()))));
}

/// A request the edge never verified carries no bearer: the deployed seam yields nothing and the
/// tool layer's not-connected refusal answers (it was `Not authenticated` before the seam).
#[test]
fn no_verified_bearer_answers_the_not_connected_refusal() {
    let service =
        temper_mcp_server::tool_service(closed(), relay(Some("http://127.0.0.1:9"), Some(SECRET)));
    let err = service.relay_client(&parts(None)).unwrap_err();
    assert_eq!(err.message, temper_mcp::seam::NOT_CONNECTED_SENTENCE);
}

/// Each misconfiguration darkens the tool door with the deployment's own sentence, naming the
/// variable — the wording is the shell's, never the tool layer's.
#[test]
fn each_misconfiguration_darkens_the_door_with_the_shells_sentence() {
    let cases = [
        (relay(None, Some(SECRET)), "TEMPER_API_BASE_URL is unset"),
        (
            relay(Some("http://x"), None),
            "TEMPER_MCP_SERVICE_SECRET is unset",
        ),
        (
            relay(Some("  "), Some(SECRET)),
            "TEMPER_API_BASE_URL is unset",
        ),
        (
            relay(Some("http://x"), Some("a-secret-with-a\u{7f}-delete")),
            "TEMPER_MCP_SERVICE_SECRET is not a valid header value",
        ),
    ];
    for (relay, names) in cases {
        let DeployedRelay::Unavailable(sentence) = relay else {
            panic!("expected a dark door naming {names}");
        };
        assert!(sentence.contains(names), "{sentence}");
        let service =
            temper_mcp_server::tool_service(closed(), DeployedRelay::Unavailable(sentence));
        let err = service.relay_client(&parts(Some(BEARER))).unwrap_err();
        assert_eq!(err.message, sentence);
        assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    }
}

/// The relay is sized to this function's budget: 45 s, under Vercel's 60 s `maxDuration`.
#[test]
fn the_relay_timeout_is_the_functions_budget() {
    assert_eq!(RELAY_REQUEST_TIMEOUT, std::time::Duration::from_secs(45));
    let DeployedRelay::Ready { config, .. } = relay(Some("http://api.test/"), Some(SECRET)) else {
        panic!("both variables set: the relay is ready");
    };
    assert_eq!(config.request_timeout, RELAY_REQUEST_TIMEOUT);
    assert_eq!(config.api_base_url, "http://api.test/");
}
