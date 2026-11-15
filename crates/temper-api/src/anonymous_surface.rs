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
//! 2. Buffers the body of every request carrying all three `X-Temper-*` signature headers, up to
//!    [`MAX_BODY_BYTES`], on every path, before routing ([`equalise_signed_body`]). A signature
//!    cannot be checked without the body, so the signature gates read it before they decide; read
//!    there and nowhere else, the read itself told a prober which paths are doors (a declared
//!    `Content-Length` with no body stalled only there, and `Expect: 100-continue` got its `100`
//!    only there). Read here, every path waits alike and continues alike for the same request.
//!    Over the cap, unreadable, or not in by [`SIGNED_BODY_DEADLINE`], it is the uniform refusal
//!    (`hmac_body`) on every path alike.
//! 3. Answers a CORS preflight from an admitted origin, identically on every path, with the
//!    security headers the per-endpoint layers would have added. Every other `OPTIONS` is routed
//!    like any other method, so it meets the route's own gate.
//! 4. Adds the CORS actual-response headers (which depend only on `Origin`) to everything else,
//!    the routed `OPTIONS` included. The layer would *answer* a routed `OPTIONS` rather than add
//!    headers to it, so for that one method the headers are taken from the layer's answer to a
//!    `GET` carrying the same request headers ([`CorsHeadersOnly`]).
//! 5. While the bearer key store is failing, answers every pre-identity refusal (a response marked
//!    [`PreIdentityRefusal`]) with the "authentication service unavailable" bytes instead, whatever
//!    its path or cause. Unavailability is server-wide state: answered only where a bearer is
//!    verified, it would tell a prober which paths verify one. Every such refusal first joins the
//!    store's refresh when its keys are stale ([`JwksKeyStore::refresh_within`]), for at most
//!    [`JwksKeyStore::REFRESH_WAIT`] from the request's arrival whatever its door, so an instance
//!    whose routes never verify a bearer still learns of an outage, a failing store recovers
//!    without bearer traffic, and the refresh runs inside a live request.
//! 6. Sets `x-request-id`, and removes `Allow` from every response that is not a 405.
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

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower::{Layer as _, Service, ServiceExt as _};

use temper_core::internal_sig::{
    MAX_BODY_BYTES, NONCE_HEADER, SIGNATURE_V2_HEADER, TIMESTAMP_HEADER,
};
use temper_services::cors::{Cors, CorsPolicy};
use temper_services::error::{unavailable_response, ApiError, PreIdentityRefusal, UnauthCause};
use temper_services::state::JwksKeyStore;
use temper_services::transport::{insert_security_headers, RequestId, REQUEST_ID_HEADER};

/// The most of a caller's own `x-request-id` the root span keeps. It is caller-chosen bytes, so
/// it is bounded before it reaches a log line, like every other echoed header.
const MAX_INBOUND_REQUEST_ID_BYTES: usize = 128;

/// A caller-supplied `x-request-id`, kept only so an operator can match it to the caller's own
/// logs. Never echoed, never used as the request's id.
#[derive(Clone, Debug)]
pub(crate) struct InboundRequestId(pub(crate) String);

/// Wrap a finished router in the outer service.
pub(crate) fn wrap(inner: Router, cors: CorsPolicy, jwks: Arc<JwksKeyStore>) -> Router {
    let layer = cors.layer();
    Router::new().fallback_service(AnonymousSurface {
        preflight: layer.layer(NeverCalled),
        headers_only: layer.layer(CorsHeadersOnly),
        actual: layer.layer(inner.clone()),
        inner,
        cors: Arc::new(cors),
        jwks,
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
    /// Joined on every pre-identity refusal, then read through `is_failing` for the unavailable
    /// swap.
    jwks: Arc<JwksKeyStore>,
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
        // Read before routing: a bearer lookup inside may run the refresh that clears it, and a
        // refusal's answer must not depend on whether its door looked one up.
        let was_failing = self.jwks.is_failing();
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

        // Before routing decides which gate runs, so the read cannot depend on the path.
        let request = equalise_signed_body(request).await;
        // The refusal's refresh wait is counted from here, once any signed body is in: a bearer
        // lookup counts its own wait from when it starts, which for a signed request is after the
        // body, so a clock started before the read would let a slow signed body hold the doors
        // that look a bearer up longer than the doors that do not.
        let entered = tokio::time::Instant::now();
        let request = match request {
            Ok(request) => request,
            Err(headers) => {
                // Refused before routing, so no endpoint layer adds what a routed refusal carries:
                // the security headers and the CORS actual-response headers are added here.
                let mut refusal = ApiError::Unauthenticated(UnauthCause::HmacBody).into_response();
                insert_security_headers(refusal.headers_mut());
                let mut view = Request::new(Body::empty());
                *view.method_mut() = Method::GET;
                *view.headers_mut() = headers;
                let Ok(cors) = self.headers_only.clone().oneshot(view).await;
                for (name, value) in cors.headers() {
                    refusal.headers_mut().append(name, value.clone());
                }
                return self.finish_answer(refusal, &id, entered, was_failing).await;
            }
        };

        let Ok(response) = if self
            .cors
            .is_admitted_preflight(request.method(), request.headers())
        {
            // `tower_http` answers every `OPTIONS` itself and never calls inward, so the inner
            // service here is never reached; it exists only to give the layer a type to wrap.
            // Answered before routing, so before any span: this event is the line its request id
            // joins to.
            tracing::debug!(request_id = id.as_str(), "cors preflight answered");
            self.preflight
                .clone()
                .oneshot(request)
                .await
                .map(|mut response| {
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
            let Ok(cors) = self.headers_only.clone().oneshot(view).await;
            self.inner
                .clone()
                .oneshot(request)
                .await
                .map(|mut response| {
                    // The empty answer carries nothing but what the layer added.
                    for (name, value) in cors.headers() {
                        response.headers_mut().append(name, value.clone());
                    }
                    response
                })
        } else {
            self.actual.clone().oneshot(request).await
        };
        self.finish_answer(response, &id, entered, was_failing)
            .await
    }

    /// Every answer's tail, routed or refused before routing: a pre-identity refusal joins the key
    /// store's refresh, is logged, and is swapped while the store is failing; then [`finish`].
    async fn finish_answer(
        &self,
        mut response: Response,
        id: &RequestId,
        entered: tokio::time::Instant,
        was_failing: bool,
    ) -> Response {
        if let Some(PreIdentityRefusal(cause)) = response.extensions().get().copied() {
            // Join the store's refresh if its keys are stale, until the same deadline from the
            // request's arrival for every door (a bearer lookup that already waited has used it
            // up). "Unavailable" if the store was failing when the request arrived, is failing
            // now, or still has no usable keys: a refusal whose wait saw the endpoint recover, or
            // that gave up on a slow fetch, still tells its caller to retry rather than that its
            // credential was refused.
            let usable = self
                .jwks
                .refresh_within(entered + JwksKeyStore::REFRESH_WAIT)
                .await;
            let unavailable = was_failing || self.jwks.is_failing() || !usable;
            // The one line an operator joins a caller's quoted `x-request-id` to. Emitted here, not
            // under the root span, so it carries the id whatever the log filter does to spans.
            tracing::warn!(
                request_id = id.as_str(),
                cause = cause.as_str(),
                unavailable,
                status_code = 401,
                "auth error"
            );
            if unavailable {
                response = unavailable_if_refused(response);
            }
        }
        finish(&mut response, id);
        response
    }
}

/// How long a signed request's body may take to arrive once its headers have.
///
/// Without one, a caller that sends the signature headers and then drips its body a byte at a
/// time holds a task and up to [`MAX_BODY_BYTES`] for as long as it likes, and since the read now
/// happens on every path it could do so on every path. A real signer sends a body under a kilobyte
/// in one write, so ten seconds is far past any honest delivery. The same deadline on every path,
/// from the same point, keeps the wait equal: an expired read is the same refusal everywhere.
const SIGNED_BODY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a request carries all three signature headers. Presence only: whether they parse is the
/// gate's question, and a gate is not reached before routing.
fn carries_signature(headers: &HeaderMap) -> bool {
    [TIMESTAMP_HEADER, NONCE_HEADER, SIGNATURE_V2_HEADER]
        .iter()
        .all(|name| headers.contains_key(*name))
}

/// The signed-body equaliser: a request carrying the signature headers has its body read into
/// memory, up to [`MAX_BODY_BYTES`], and put back as a buffered body; any other request passes
/// untouched, and its body is read (or not) by whatever it reaches.
///
/// The request is the only input, never its path, so every path waits for the same body and sends
/// `100 Continue` for the same request. A declared length over the cap is refused without reading
/// it (and so without `100 Continue`); a body that turns out larger, cannot be read, or has not
/// arrived by [`SIGNED_BODY_DEADLINE`] is refused when that is known.
///
/// The cap counts the bytes on the wire. Request decompression runs inside routing, so a
/// compressed body is buffered compressed, and a signature gate's own read, after decompression,
/// counts the decompressed bytes against the same constant. The refusal hands
/// back the request's headers, which the caller needs for its CORS answer.
///
/// It adds no read a caller could not already force: any signed request reached a door's gate and
/// was read to this same cap there.
async fn equalise_signed_body(request: Request) -> Result<Request, HeaderMap> {
    if !carries_signature(request.headers()) {
        return Ok(request);
    }
    let declared_over = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|len| len > MAX_BODY_BYTES as u64);
    let (parts, body) = request.into_parts();
    if declared_over {
        return Err(parts.headers);
    }
    match tokio::time::timeout(
        SIGNED_BODY_DEADLINE,
        axum::body::to_bytes(body, MAX_BODY_BYTES),
    )
    .await
    {
        Ok(Ok(bytes)) => Ok(Request::from_parts(parts, Body::from(bytes))),
        Ok(Err(_)) | Err(_) => Err(parts.headers),
    }
}

/// A pre-identity refusal re-rendered as the "unavailable" bytes. Its headers (CORS, the security
/// set) stay, since they depend on the request and not the cause; only the body and its length
/// change. Every other response passes through untouched.
fn unavailable_if_refused(response: Response) -> Response {
    if response.extensions().get::<PreIdentityRefusal>().is_none() {
        return response;
    }
    let (mut parts, _) = response.into_parts();
    let (_, body) = unavailable_response().into_parts();
    // The new body's exact size frames it; the old length would not.
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, body)
}

/// The two rules every answer leaves under: the request id, and `Allow` only on a 405.
fn finish(response: &mut Response, id: &RequestId) {
    let headers = response.headers_mut();
    headers.insert(
        REQUEST_ID_HEADER,
        HeaderValue::from_str(id.as_str()).expect("a request id is hex"),
    );
    if response.status() != StatusCode::METHOD_NOT_ALLOWED {
        response.headers_mut().remove(header::ALLOW);
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
    use temper_services::error::{ApiError, UnauthCause};

    // FAILS IF: the unavailable swap touches a response that is not a marked pre-identity
    // refusal (a verified-identity 401 such as `account is deactivated` would turn retryable), or
    // misses a marked one.
    #[tokio::test]
    async fn only_a_marked_refusal_is_swapped() {
        let verified = ApiError::Unauthorized("account is deactivated".to_string()).into_response();
        let kept = unavailable_if_refused(verified);
        let body = axum::body::to_bytes(kept.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("account is deactivated"));

        let marked = ApiError::Unauthenticated(UnauthCause::MissingBearer).into_response();
        let swapped = unavailable_if_refused(marked);
        assert_eq!(swapped.headers().get(header::CONTENT_LENGTH), None);
        let body = axum::body::to_bytes(swapped.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Authentication service unavailable"));
    }
}
