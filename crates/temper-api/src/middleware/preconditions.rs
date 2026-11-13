//! Refuse a write precondition this server does not evaluate.
//!
//! `If-Match`, `If-None-Match` and `If-Unmodified-Since` ask the server to apply a write only if
//! the target is in a state the client names: still at a version it read, absent
//! (`If-None-Match: *`, create-if-absent), or unchanged since a time. A server that ignores them
//! applies the write anyway, so the client's guard is silently gone. That is worse than no support
//! at all: the client believes it is protected. The rule here is that a precondition the server
//! cannot honour is refused with `400` under `PRECONDITION_NOT_HONOURED`, never dropped.
//!
//! # Scope
//!
//! [`refuse_unhonoured_preconditions`] is applied once, by `routes::apply_transport_layers`, to
//! every route of both temper-api app builders (`create_app` and `create_internal_app`), so a new
//! route there is covered without anyone remembering to add it. Outside it, and honouring none of
//! these headers: the MCP function (`POST`/`DELETE /mcp`, `POST /oauth/register`, built by
//! temper-mcp-server) and the Node OAuth functions in `packages/temper-cloud`.
//!
//! The layer is keyed on the method, not on the route: any method that is not safe (`GET`, `HEAD`,
//! `OPTIONS`, `TRACE`) is treated as a write, extension methods included. Safe methods pass
//! untouched with or without the headers. The value is never read: a header that is present is
//! refused whatever it says, empty, `*` or unparseable alike.
//!
//! The method is the whole test, deliberately. A few doors change state on `GET` (the Vercel
//! crons, which mount one handler on `GET` and `POST`, and the Slack link callback), and their
//! `GET` is not policed: a cron or an OAuth redirect sends no precondition, and policing a read by
//! route would make this layer a list of routes, which is the thing that can miss one. Their
//! `POST` is policed like any other write.
//!
//! # Placement
//!
//! It is added to the mounted routes before the fallback and the base layers exist. That makes it
//! outside every tier's auth stack, so the refusal costs no lookup and depends on nothing about the
//! caller, and inside the security headers, the root span and CORS, so the refusal carries them
//! like any other answer. An unmatched path still meets the fallback's `404`. A request carrying a
//! policed header with a method its path does not mount is refused here (`400`), before the method
//! router's `405`; an anonymous one is refused before authentication's `401`. The integration test
//! `the_refusal_carries_cors_and_the_security_headers_and_misses_no_fallback` pins the placement.
//!
//! # Honouring a header
//!
//! A route that starts evaluating one of these headers is listed in `HONOURED`, and from then on
//! that header reaches its handler on that route. Nothing is listed yet. Each entry owes two
//! witnesses against its handler: a mismatched precondition is refused with `412` and nothing
//! applied, and a value the handler cannot parse is refused with `400`, never treated as absent.

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use temper_services::error::ApiError;

/// The precondition headers this layer polices, in the order a refusal names them: when a request
/// carries several, the first in this list is the one named.
pub const POLICED: [HeaderName; 3] = [
    axum::http::header::IF_MATCH,
    axum::http::header::IF_NONE_MATCH,
    axum::http::header::IF_UNMODIFIED_SINCE,
];

/// A route that evaluates a policed header itself: the method, the route template exactly as it
/// is mounted (`/api/resources/{id}`), and the header it honours.
struct Honoured {
    method: Method,
    route: &'static str,
    header: HeaderName,
}

/// The routes whose handlers honour a policed header. Empty: no handler evaluates one yet. An
/// entry is the whole exemption; the handler it names must then honour the header, because from
/// here on nothing else refuses it. See the module doc for the witnesses an entry owes.
const HONOURED: &[Honoured] = &[];

/// True for the methods that do not change state, which this layer never inspects.
fn is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

fn is_honoured(method: &Method, route: Option<&str>, header: &HeaderName) -> bool {
    let Some(route) = route else {
        return false;
    };
    HONOURED
        .iter()
        .any(|h| h.method == *method && h.route == route && h.header == *header)
}

/// The header as the refusal names it.
fn display_name(header: &HeaderName) -> &str {
    match *header {
        axum::http::header::IF_MATCH => "If-Match",
        axum::http::header::IF_NONE_MATCH => "If-None-Match",
        axum::http::header::IF_UNMODIFIED_SINCE => "If-Unmodified-Since",
        _ => header.as_str(),
    }
}

/// The refusal for one header. Names the header and never its value: the value is the client's,
/// and the refusal is about the header being present at all.
fn refusal(header: &HeaderName) -> ApiError {
    let name = display_name(header);
    ApiError::PreconditionNotHonoured(format!(
        "the {name} header is not honoured on this route. The server does not evaluate it, so the \
         request is refused rather than applied unconditionally. Nothing was applied; resend \
         without the header to write unconditionally"
    ))
}

/// The layer: refuse a write that carries a policed header its route does not honour.
pub async fn refuse_unhonoured_preconditions(request: Request, next: Next) -> Response {
    let method = request.method();
    if !is_safe(method) {
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(MatchedPath::as_str);
        if let Some(header) = POLICED.iter().find(|header| {
            request.headers().contains_key(*header) && !is_honoured(method, route, header)
        }) {
            return refusal(header).into_response();
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::routing::any;
    use axum::Router;
    use tower::ServiceExt;

    /// A router whose one route accepts every method and answers `204`, behind the layer.
    fn app() -> Router {
        Router::new()
            .route("/thing/{id}", any(|| async { StatusCode::NO_CONTENT }))
            .layer(axum::middleware::from_fn(refuse_unhonoured_preconditions))
    }

    /// Send one request; return the status and the error's code and message, if any.
    async fn send(method: &str, headers: &[(&str, &str)]) -> (StatusCode, String, String) {
        let mut request = axum::http::Request::builder()
            .method(Method::from_bytes(method.as_bytes()).expect("method"))
            .uri("/thing/1");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = app()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (
            status,
            body["error"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            body["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    }

    fn names(message: &str, header: &str) -> bool {
        message.contains(&format!(
            "the {header} header is not honoured on this route"
        ))
    }

    // FAILS IF: a refusal names the wrong header, or answers under any code but its own.
    #[test]
    fn refusal_names_its_own_header() {
        for (header, name) in
            POLICED
                .iter()
                .zip(["If-Match", "If-None-Match", "If-Unmodified-Since"])
        {
            let message = refusal(header).to_string();
            assert!(names(&message, name), "{message}");
        }
    }

    // FAILS IF: the layer reads a header's value — so that `*`, an empty value or an unparseable
    // date slips through as "absent" — or echoes the value back.
    #[tokio::test]
    async fn any_value_is_refused_and_none_is_echoed() {
        let cases = [
            ("if-match", "*"),
            ("if-match", ""),
            ("if-match", "\"value-sentinel\""),
            ("if-none-match", "*"),
            ("if-none-match", ""),
            ("if-unmodified-since", "not a date at all"),
            ("if-unmodified-since", ""),
        ];
        for (header, value) in cases {
            let (status, code, message) = send("POST", &[(header, value)]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{header}: {value:?}");
            assert_eq!(code, "PRECONDITION_NOT_HONOURED", "{header}: {value:?}");
            if !value.is_empty() && value != "*" {
                assert!(
                    !message.contains(value),
                    "{header}: echoed {value:?}: {message}"
                );
            }
        }
    }

    // FAILS IF: several headers at once name anything but the first in `POLICED`'s order.
    #[tokio::test]
    async fn several_headers_name_the_first_in_policed_order() {
        let all = [
            ("if-unmodified-since", "Wed, 21 Oct 2015 07:28:00 GMT"),
            ("if-none-match", "*"),
            ("if-match", "\"a\""),
        ];
        let (_, _, message) = send("PUT", &all).await;
        assert!(names(&message, "If-Match"), "{message}");
        let (_, _, message) = send("PUT", &all[..2]).await;
        assert!(names(&message, "If-None-Match"), "{message}");
    }

    // FAILS IF: an extension method, or a method in the wrong case, is treated as safe. Methods are
    // case-sensitive, so `post` is an extension method, not POST, and is refused as a write.
    #[tokio::test]
    async fn every_non_safe_method_is_policed() {
        for method in ["POST", "PUT", "PATCH", "DELETE", "PROPFIND", "post", "get"] {
            let (status, code, _) = send(method, &[("if-match", "\"a\"")]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method}");
            assert_eq!(code, "PRECONDITION_NOT_HONOURED", "{method}");
            let (status, _, _) = send(method, &[]).await;
            assert_eq!(status, StatusCode::NO_CONTENT, "{method} without a header");
        }
    }

    // FAILS IF: a safe method is refused for carrying a policed header.
    #[tokio::test]
    async fn safe_methods_pass_with_every_header() {
        for method in ["GET", "HEAD", "OPTIONS", "TRACE"] {
            for header in &POLICED {
                let (status, _, _) = send(method, &[(header.as_str(), "*")]).await;
                assert_eq!(status, StatusCode::NO_CONTENT, "{method} with {header}");
            }
        }
    }

    // FAILS IF: an entry is added to `HONOURED` without updating this test, and the witnesses the
    // module doc requires for it.
    #[test]
    fn nothing_is_honoured_yet() {
        assert!(HONOURED.is_empty());
        for header in &POLICED {
            assert!(!is_honoured(
                &Method::PATCH,
                Some("/api/resources/{id}"),
                header
            ));
        }
    }
}
