//! The service outside routing: what every caller's answer passes through last.
//!
//! **Why it cannot be a `Router::layer`.** `Router::layer` wraps each endpoint, and axum stamps a
//! path's `Allow` header onto the method fallback's answer *after* every such layer has returned
//! (`MethodRouter::call_with_state`, then `set_allow_header` on the top-level route future). So no
//! layer temper adds that way can remove `Allow`, and a CORS layer added that way answers every
//! `OPTIONS` 200 with the path's methods beside it: an anonymous route map. Only a service that
//! wraps the finished router sees the final response.
//!
//! **How it is wrapped.** The finished, state-bound router is the only thing in an outer router,
//! as its fallback service ([`wrap`]). The outer router has no routes, so it stamps nothing and sets
//! no `MatchedPath`; every consumer of `create_app` keeps the `Router` type it already had.
//!
//! **What it does, in order:**
//! 1. Mints the request id ([`RequestId`]) and plants it for the root span. An inbound
//!    `x-request-id` is never adopted or echoed; it is kept, bounded, only as a span field.
//! 2. Answers a CORS preflight from an admitted origin, identically on every path, with the
//!    security headers the per-endpoint layers would have added. Every other `OPTIONS` is routed
//!    like any other method, so it meets the route's own gate.
//! 3. Adds the CORS actual-response headers (which depend only on `Origin`) to everything else,
//!    the routed `OPTIONS` included. The layer would *answer* a routed `OPTIONS` rather than add
//!    headers to it, so for that one method the headers are taken from the layer's answer to a
//!    `GET` carrying the same request headers ([`CorsHeadersOnly`]).
//! 4. Sets `x-request-id`, and removes `Allow` from every response that is not a 405.
//!
//! The strip is keyed "unless 405", never "on 401": request decompression wraps every tier gate,
//! so an undecodable `Content-Encoding` is a 415 before any gate runs, and on an unmounted method
//! axum stamps the method map onto that 415 too. RFC 9110 requires `Allow` on a 405 and makes it
//! optional elsewhere.
//!
//! **`HEAD` and `content-length`.** The outer router's fallback is a second top-level route, and
//! axum's top-level route future inserts `content-length` from the body's exact size when the
//! header is absent. The inner route has already emptied a `HEAD` body, so an inner answer that had
//! no `content-length` (a body of unknown size) would read as exactly 0 and be told so. [`finish`]
//! gives such an answer a body of unknown size, so the outer route adds nothing and the header set
//! is the inner router's own.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower::{Layer as _, Service, ServiceExt as _};

use temper_services::cors::{Cors, CorsPolicy};
use temper_services::transport::{
    insert_security_headers, unsized_empty_body, RequestId, REQUEST_ID_HEADER,
};

/// The most of a caller's own `x-request-id` the root span keeps. It is caller-chosen bytes, so
/// it is bounded before it reaches a log line, like every other echoed header.
const MAX_INBOUND_REQUEST_ID_BYTES: usize = 128;

/// A caller-supplied `x-request-id`, kept only so an operator can match it to the caller's own
/// logs. Never echoed, never used as the request's id.
#[derive(Clone, Debug)]
pub(crate) struct InboundRequestId(pub(crate) String);

/// Wrap a finished router in the outer service.
pub(crate) fn wrap(inner: Router, cors: CorsPolicy) -> Router {
    let layer = cors.layer();
    Router::new().fallback_service(AnonymousSurface {
        preflight: layer.layer(NeverCalled),
        headers_only: layer.layer(CorsHeadersOnly),
        actual: layer.layer(inner.clone()),
        inner,
        cors: Arc::new(cors),
    })
}

/// The outer service. The two CORS-wrapped services are built once, here, and cloned per request
/// (each is a few `Arc`s), rather than rebuilt from the layer on every call.
#[derive(Clone)]
struct AnonymousSurface {
    /// The router, unwrapped: for an `OPTIONS` that is not an admitted preflight.
    inner: Router,
    /// The router behind CORS: every request that is not an `OPTIONS`.
    actual: Cors<Router>,
    /// The preflight answerer: CORS around a service it never calls.
    preflight: Cors<NeverCalled>,
    /// CORS around an empty answer: the actual-response headers for a routed `OPTIONS`.
    headers_only: Cors<CorsHeadersOnly>,
    cors: Arc<CorsPolicy>,
}

impl Service<Request> for AnonymousSurface {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // The inner router is always ready (axum's `Router` is), and it is cloned per call.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let this = self.clone();
        Box::pin(async move { Ok(this.handle(request).await) })
    }
}

impl AnonymousSurface {
    async fn handle(self, mut request: Request) -> Response {
        let id = RequestId::mint();
        request.extensions_mut().insert(id.clone());
        // `to_str` succeeds only on visible ASCII, so any byte offset is a cut point.
        let inbound = request
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|inbound| {
                InboundRequestId(
                    inbound[..inbound.len().min(MAX_INBOUND_REQUEST_ID_BYTES)].to_owned(),
                )
            });
        if let Some(inbound) = inbound {
            request.extensions_mut().insert(inbound);
        }
        let is_head = request.method() == Method::HEAD;

        let Ok(mut response) = if self
            .cors
            .is_admitted_preflight(request.method(), request.headers())
        {
            // `tower_http` answers every `OPTIONS` itself and never calls inward, so the inner
            // service here is never reached; it exists only to give the layer a type to wrap.
            // Answered before routing, so before any span: this event is the line its request id
            // joins to.
            tracing::debug!(request_id = id.as_str(), "cors preflight answered");
            self.preflight.oneshot(request).await.map(|mut response| {
                insert_security_headers(response.headers_mut());
                response
            })
        } else if request.method() == Method::OPTIONS {
            // Not a preflight this instance answers: an ordinary request. Routed without the CORS
            // layer, which would otherwise answer it 200 before routing could, then given the
            // same actual-response headers any other method gets from the same `Origin`.
            let mut view = Request::new(axum::body::Body::empty());
            *view.method_mut() = Method::GET;
            *view.headers_mut() = request.headers().clone();
            let Ok(cors) = self.headers_only.oneshot(view).await;
            self.inner.oneshot(request).await.map(|mut response| {
                for (name, value) in cors.headers() {
                    if name != header::CONTENT_LENGTH {
                        response.headers_mut().append(name, value.clone());
                    }
                }
                response
            })
        } else {
            self.actual.oneshot(request).await
        };

        finish(&mut response, &id, is_head);
        response
    }
}

/// The rules every answer leaves under: the request id, `Allow` only on a 405, and (for `HEAD`) no
/// `content-length` the inner router did not set (see the module doc).
fn finish(response: &mut Response, id: &RequestId, is_head: bool) {
    let headers = response.headers_mut();
    headers.insert(
        REQUEST_ID_HEADER,
        HeaderValue::from_str(id.as_str()).expect("a request id is hex"),
    );
    if response.status() != StatusCode::METHOD_NOT_ALLOWED {
        response.headers_mut().remove(header::ALLOW);
    }
    if is_head && !response.headers().contains_key(header::CONTENT_LENGTH) {
        *response.body_mut() = unsized_empty_body();
    }
}

/// The inner service of the preflight answerer. `tower_http`'s CORS layer short-circuits every
/// `OPTIONS`, and only admitted preflights (always `OPTIONS`) are sent to it, so this never runs.
/// If it ever did, it answers 500 rather than reaching the router.
#[derive(Clone, Copy)]
struct NeverCalled;

impl Service<Request> for NeverCalled {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request) -> Self::Future {
        tracing::error!(
            "CORS preflight answerer called inward; tower-http's short-circuit changed"
        );
        std::future::ready(Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response()))
    }
}

/// An empty answer for the CORS layer to decorate: how a routed `OPTIONS` gets the actual-response
/// headers without the layer short-circuiting it. Called only with a `GET` view of the request.
#[derive(Clone, Copy)]
struct CorsHeadersOnly;

impl Service<Request> for CorsHeadersOnly {
    type Response = Response;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request) -> Self::Future {
        std::future::ready(Ok(Response::new(axum::body::Body::empty())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;

    // FAILS IF: a `HEAD` answer whose body is of unknown size (no `content-length` from the inner
    // router) leaves the outer route advertising `content-length: 0`, a claim that the `GET` body
    // is empty.
    #[tokio::test]
    async fn head_of_an_unsized_body_is_not_advertised_as_empty() {
        let inner = Router::new().route("/s", get(|| async { unsized_empty_body() }));
        let app = wrap(inner, CorsPolicy::new(&[]));
        let head = app
            .clone()
            .oneshot(Request::head("/s").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers().get(header::CONTENT_LENGTH), None, "{head:?}");
        let get = app
            .oneshot(Request::get("/s").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(get.headers().get(header::CONTENT_LENGTH), None, "{get:?}");
    }
}
