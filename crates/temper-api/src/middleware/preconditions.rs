//! Refuse a write precondition this server does not evaluate.
//!
//! `If-Match`, `If-None-Match` and `If-Unmodified-Since` ask the server to apply a write only if
//! the target is in a state the client names: still at a version it read, absent
//! (`If-None-Match: *`, create-if-absent), or unchanged since a time. A server that ignores them
//! applies the write anyway, so the client's guard is silently gone. That is worse than no support
//! at all: the client believes it is protected. The rule here is that a precondition the server
//! cannot honour is refused with `400` under `PRECONDITION_NOT_HONOURED`, never dropped.
//!
//! # Who can see the refusal
//!
//! Only a caller the route already admits. An unauthenticated caller learns nothing except from
//! the designated public routes (`docs/concepts/trust-boundary.md`), so a caller the route's gate
//! refuses (no token, a bad or expired one, a deactivated account, a machine on a human tier, an
//! unapproved profile, an unsigned internal call, an unattested webhook) gets exactly the answer it
//! gets without the header: status, body and `Allow` alike.
//!
//! One consequence of axum's routing is admitted rather than refused. Where two groups share a path
//! (`/api/profile`: the person-only `PATCH` and the machine-admitting `GET`), the method router's
//! fallback for an unmounted method is the last-merged group's, so it answers behind that group's
//! stack. A machine sending `PUT /api/profile` with a header is admitted by the status group and
//! refused `400`, where without one it gets `405`. It learns nothing: `GET` on the same path already
//! admits it. The integration test pins this.
//!
//! # Scope
//!
//! Which route groups are policed is declared per group in the route table
//! (`routes::Preconditions`), never inferred from the method, and every row must declare it:
//!
//! - **`Refused`**: every group behind a layer gate (the bearer tiers and the three HMAC
//!   signature tiers). [`refuse_unhonoured_preconditions`] is applied by `routes::mount_group` to
//!   each of these groups, in both app builders.
//! - **`InHandler`**: the webhook intake, whose gate (the broker attestation) is inside its
//!   handler, so no layer can run after it. The handler calls [`refuse_unhonoured`] once the
//!   attestation verifies and its connector resolves (both are one `401`, which a header must not
//!   split), before anything is written.
//! - **`Exempt`**, each with its reason at the row:
//!   - the nine Vercel crons: none is a client write to a resource, and the scheduler sends no
//!     precondition, so the host pin and the secret answer exactly as they did before this layer;
//!   - the Slack link callback: the identity provider's redirect target, `GET` only;
//!   - `/api/health`: the designated public route, which mounts no write and keeps its own `405`.
//!
//! None of the exemptions is a client write to a resource, which is the only thing a conditional
//! header means. Outside temper-api altogether, and honouring none of these headers: the MCP
//! function (`POST`/`DELETE /mcp`, `POST /oauth/register`, built by temper-mcp-server), the Node
//! OAuth functions in `packages/temper-cloud`, and the Swagger routes (`ENABLE_SWAGGER`), which
//! are merged outside the table and mount `GET` only.
//!
//! Within a policed group the method is the test: any method that is not safe (`GET`, `HEAD`,
//! `OPTIONS`, `TRACE`) is treated as a write, extension methods included. Safe methods pass
//! untouched with or without the headers. The value is never read: a header that is present is
//! refused whatever it says, empty, `*` or unparseable alike.
//!
//! # Placement
//!
//! In a `Refused` group the layer is inside every gate of the tier stack: it runs after
//! `require_auth`, `refuse_machine`, `require_system_access` and the signature gate, and before
//! the handler and its extractors, so a refusal reads no row and differs by nothing about the
//! target. It is added with `Router::layer`, so it also wraps the method router's fallback: an
//! admitted caller sending a policed header with a method the path does not mount gets `400`, with
//! the fallback's `Allow`. It sits inside the security headers, the root span and CORS, so the
//! refusal carries them like any other answer. An unmatched path never reaches it and meets the
//! fallback's `404`. `tests/unhonoured_precondition_test.rs` pins all of this, the anonymous answers
//! included.
//!
//! # Honouring a header
//!
//! A route that starts evaluating one of these headers is listed in `HONOURED`, and from then on
//! that header reaches its handler on that route. Nothing is listed yet. An entry must name a
//! documented route mounted by a `Refused` group (`routes::tests` holds this): an exempt route is
//! not policed at all, and an `InHandler` door honours nothing. Each entry owes witnesses against
//! its handler: a mismatched precondition is refused with `412` and nothing applied; a value the
//! handler cannot parse is refused with `400`, never treated as absent; a caller who cannot see
//! the target gets the masked `404`, never `412` (the handler applies visibility before it compares,
//! or `412` against `404` is an existence oracle); and an unauthenticated caller sending the header
//! gets the identical `401`.

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderMap, HeaderName, Method};
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
pub(crate) struct Honoured {
    pub(crate) method: Method,
    pub(crate) route: &'static str,
    pub(crate) header: HeaderName,
}

/// The routes whose handlers honour a policed header. Empty: no handler evaluates one yet. An
/// entry is the whole exemption; the handler it names must then honour the header, because from
/// here on nothing else refuses it. See the module doc for the witnesses an entry owes.
pub(crate) const HONOURED: &[Honoured] = &[];

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

/// The one decision: refuse a non-safe request carrying a policed header that `route` does not
/// honour. Shared by the layer and [`refuse_unhonoured`], so the two cannot answer differently.
fn check(method: &Method, route: Option<&str>, headers: &HeaderMap) -> Result<(), ApiError> {
    if is_safe(method) {
        return Ok(());
    }
    match POLICED
        .iter()
        .find(|header| headers.contains_key(*header) && !is_honoured(method, route, header))
    {
        Some(header) => Err(refusal(header)),
        None => Ok(()),
    }
}

/// The layer, for a `Refused` group: refuse a write that carries a policed header its route does
/// not honour. Runs innermost in the tier stack; see the module doc's placement section.
pub async fn refuse_unhonoured_preconditions(request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    if let Err(refusal) = check(request.method(), route, request.headers()) {
        return refusal.into_response();
    }
    next.run(request).await
}

/// The same refusal, for an `InHandler` door: called by the handler once its own gate has
/// admitted the caller, before it writes anything. Such a door honours no header (`HONOURED` may
/// only name a `Refused` route), so the route is not consulted.
pub fn refuse_unhonoured(method: &Method, headers: &HeaderMap) -> Result<(), ApiError> {
    check(method, None, headers)
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

    // FAILS IF: the handler-callable check answers differently from the layer: a safe method
    // refused, a write with a policed header let through, or a refusal under another code.
    #[test]
    fn the_handler_check_agrees_with_the_layer() {
        for header in &POLICED {
            let mut headers = HeaderMap::new();
            headers.insert(header.clone(), "*".parse().expect("value"));
            for method in [Method::GET, Method::HEAD, Method::OPTIONS, Method::TRACE] {
                assert!(refuse_unhonoured(&method, &headers).is_ok(), "{method}");
            }
            for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                let refused = refuse_unhonoured(&method, &headers).expect_err("refused");
                assert!(
                    matches!(refused, ApiError::PreconditionNotHonoured(_)),
                    "{method}: {refused:?}"
                );
            }
        }
        assert!(refuse_unhonoured(&Method::POST, &HeaderMap::new()).is_ok());
    }
}
