//! Work the host runs without holding the response: how a request's span flush leaves the response
//! path.
//!
//! ## Why the flush must not hold the response
//!
//! [`traced_request`](crate::traced_request) has to flush a request's span before the sandbox
//! freezes, and the span only exists to flush once the response is built. Done inline, the client
//! waits for that flush: up to `FLUSH_BUDGET`, 500 ms, on every request whose export is slow. When
//! the collector is slow, that is most requests, each held for telemetry the client never sees.
//!
//! A serverless host that can keep an invocation alive after its response removes the trade. Vercel
//! can: `vercel_runtime::AppState::wait_until` starts the registered future at once, alongside the
//! response, and delays the invocation's end until it settles, bounded by the function's
//! `maxDuration`. The flush still finishes before the freeze, and nobody waits for it. The cost and
//! the limits of that are on `export::flush_after_response`.
//!
//! ## The seam
//!
//! [`AfterResponse`] is a request extension: "run this without holding the response". A host that
//! can do that inserts one; [`traced_request`](crate::traced_request) hands it the flush when it is
//! present and flushes inline, within the budget, when it is not. The inline path serves the
//! long-lived `temper-api` binary, whose process does not freeze, and any host that installs no
//! handle. A panicking request always flushes inline: the resumed unwind ends the connection's task
//! on Vercel before the runtime waits on the request's background work (see `traced_request`).
//!
//! This crate defines the handle without depending on a host. The Vercel adapter that inserts it is
//! [`vercel::AfterResponseLayer`], behind the `vercel` feature, so a consumer that is not on Vercel
//! does not compile `vercel_runtime`.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A future the host runs without holding the response.
pub type AfterResponseFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// A request extension that runs work without holding the response.
///
/// The host that inserts it keeps the invocation alive until the future settles, for any request
/// that returns a response. A host that cannot do that must not insert one: the flush handed to it
/// is what gets spans out before a freeze.
#[derive(Clone)]
pub struct AfterResponse(Arc<dyn Fn(AfterResponseFuture) + Send + Sync>);

impl AfterResponse {
    /// A handle that passes each future to `run`.
    pub fn new<F>(run: F) -> Self
    where
        F: Fn(AfterResponseFuture) + Send + Sync + 'static,
    {
        Self(Arc::new(run))
    }

    /// Hand `future` to the host.
    pub(crate) fn run(&self, future: AfterResponseFuture) {
        (self.0)(future);
    }
}

impl fmt::Debug for AfterResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AfterResponse")
    }
}

/// The Vercel adapter: an [`AfterResponse`] backed by `AppState::wait_until`.
#[cfg(feature = "vercel")]
pub mod vercel {
    use std::task::{Context, Poll};

    use super::AfterResponse;

    /// Insert an [`AfterResponse`] into each request, backed by that request's
    /// `vercel_runtime::AppState::wait_until`.
    ///
    /// Goes **outside** `vercel_runtime::axum::VercelLayer`, which receives the per-request
    /// `AppState` and discards it:
    ///
    /// ```ignore
    /// ServiceBuilder::new()
    ///     .layer(AfterResponseLayer)
    ///     .layer(VercelLayer::new())
    ///     .service(app)
    /// ```
    ///
    /// `VercelLayer` rebuilds the request from its parts, so the extension reaches the axum router.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct AfterResponseLayer;

    impl<S> tower::Layer<S> for AfterResponseLayer {
        type Service = AfterResponseService<S>;

        fn layer(&self, inner: S) -> Self::Service {
            AfterResponseService { inner }
        }
    }

    /// The service [`AfterResponseLayer`] builds.
    #[derive(Debug, Clone)]
    pub struct AfterResponseService<S> {
        inner: S,
    }

    impl<S> tower::Service<(vercel_runtime::AppState, vercel_runtime::Request)>
        for AfterResponseService<S>
    where
        S: tower::Service<(vercel_runtime::AppState, vercel_runtime::Request)>,
    {
        type Response = S::Response;
        type Error = S::Error;
        type Future = S::Future;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(cx)
        }

        fn call(
            &mut self,
            (state, mut request): (vercel_runtime::AppState, vercel_runtime::Request),
        ) -> Self::Future {
            // Each request carries its own `AppState`, and its `end` message waits on that state's
            // background work. So the handle must be built from this request's state, never shared.
            let host = state.clone();
            request
                .extensions_mut()
                .insert(AfterResponse::new(move |future| host.wait_until(future)));
            self.inner.call((state, request))
        }
    }
}
