//! Refuse a write precondition this server does not evaluate.
//!
//! `If-Match` and `If-Unmodified-Since` ask the server to apply a write only if the target is
//! still in a state the client names. A server that ignores them applies the write anyway, so the
//! client's guard against a lost update is silently gone. That is worse than no support at all:
//! the client believes it is protected. The rule here is that a precondition the server cannot
//! honour is refused with `400`, never dropped.
//!
//! [`refuse_unhonoured_preconditions`] is applied once, to every route, by
//! `routes::apply_transport_layers` — the function both app builders share — so a new route is
//! covered without anyone remembering to add it. It is keyed on the method, not on the route: any
//! method that is not safe (`GET`, `HEAD`, `OPTIONS`, `TRACE`) is treated as a write. Safe methods
//! pass untouched with or without the headers, and `If-None-Match` is never examined.
//!
//! It sits outside every tier's auth stack, so the refusal costs no lookup and depends on nothing
//! about the caller: the answer is about the request's shape, not about any resource.
//!
//! A route that starts evaluating one of these headers is listed in `HONOURED`, and from then on
//! the header reaches its handler. Nothing is listed yet.

use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use temper_services::error::ApiError;

/// The precondition headers this layer polices, in the order a refusal names them.
pub const POLICED: [HeaderName; 2] = [
    axum::http::header::IF_MATCH,
    axum::http::header::IF_UNMODIFIED_SINCE,
];

/// A route that evaluates a policed header itself: the method, the route template exactly as it
/// is mounted (`/api/resources/{id}`), and the header it honours.
struct Honoured {
    method: Method,
    route: &'static str,
    header: HeaderName,
}

/// The routes whose handlers honour a policed header. Empty: no handler evaluates either one yet.
/// An entry is the whole exemption; the handler it names must then honour the header, because
/// from here on nothing else refuses it.
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

/// The refusal for one header. Names the header and never its value: the value is the client's,
/// and the refusal is about the header being present at all.
fn refusal(header: &HeaderName) -> ApiError {
    let name = match *header {
        axum::http::header::IF_MATCH => "If-Match",
        axum::http::header::IF_UNMODIFIED_SINCE => "If-Unmodified-Since",
        _ => header.as_str(),
    };
    ApiError::BadRequest(format!(
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

    #[test]
    fn refusal_names_the_header_and_not_its_value() {
        for header in &POLICED {
            let message = refusal(header).to_string();
            assert!(
                message.contains("If-Match") || message.contains("If-Unmodified-Since"),
                "{message}"
            );
            assert!(message.contains("not honoured on this route"), "{message}");
        }
    }

    #[test]
    fn nothing_is_honoured_yet() {
        for header in &POLICED {
            assert!(!is_honoured(
                &Method::PATCH,
                Some("/api/resources/{id}"),
                header
            ));
        }
    }
}
