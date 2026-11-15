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
//! 3. Adds the CORS actual-response headers (which depend only on `Origin`) to everything else.
//! 4. Sets `x-request-id`, and removes `Allow` from every response that is not a 405.
//!
//! The strip is keyed "unless 405", never "on 401": request decompression wraps every tier gate,
//! so an undecodable `Content-Encoding` is a 415 before any gate runs, and on an unmounted method
//! axum stamps the method map onto that 415 too. RFC 9110 requires `Allow` on a 405 and makes it
//! optional elsewhere.

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

use temper_services::cors::CorsPolicy;
use temper_services::transport::{insert_security_headers, RequestId, REQUEST_ID_HEADER};

/// The most of a caller's own `x-request-id` the root span keeps. It is caller-chosen bytes, so
/// it is bounded before it reaches a log line, like every other echoed header.
const MAX_INBOUND_REQUEST_ID_BYTES: usize = 128;

/// A caller-supplied `x-request-id`, kept only so an operator can match it to the caller's own
/// logs. Never echoed, never used as the request's id.
#[derive(Clone, Debug)]
pub(crate) struct InboundRequestId(pub(crate) String);

/// Wrap a finished router in the outer service.
pub(crate) fn wrap(inner: Router, cors: CorsPolicy) -> Router {
    Router::new().fallback_service(AnonymousSurface {
        inner,
        cors: Arc::new(cors),
    })
}

#[derive(Clone)]
struct AnonymousSurface {
    inner: Router,
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
        let inbound = request
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|inbound| {
                let mut cut = inbound.len().min(MAX_INBOUND_REQUEST_ID_BYTES);
                while !inbound.is_char_boundary(cut) {
                    cut -= 1;
                }
                InboundRequestId(inbound[..cut].to_owned())
            });
        if let Some(inbound) = inbound {
            request.extensions_mut().insert(inbound);
        }

        let mut response = if self
            .cors
            .is_admitted_preflight(request.method(), request.headers())
        {
            // `tower_http` answers every `OPTIONS` itself and never calls inward, so the inner
            // service here is never reached; it exists only to give the layer a type to wrap.
            let mut response =
                into_infallible(self.cors.layer().layer(NeverCalled).oneshot(request).await);
            insert_security_headers(response.headers_mut());
            response
        } else if request.method() == Method::OPTIONS {
            // Not a preflight this instance answers: an ordinary request. Routed without the CORS
            // layer, which would otherwise answer it 200 before routing could.
            into_infallible(self.inner.oneshot(request).await)
        } else {
            into_infallible(self.cors.layer().layer(self.inner).oneshot(request).await)
        };

        finish(&mut response, &id);
        response
    }
}

/// The two response rules every answer leaves under: the request id, and `Allow` only on a 405.
fn finish(response: &mut Response, id: &RequestId) {
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(id.as_str()) {
        headers.insert(REQUEST_ID_HEADER, value);
    }
    if response.status() != StatusCode::METHOD_NOT_ALLOWED {
        response.headers_mut().remove(header::ALLOW);
    }
}

fn into_infallible(result: Result<Response, Infallible>) -> Response {
    match result {
        Ok(response) => response,
        Err(never) => match never {},
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
