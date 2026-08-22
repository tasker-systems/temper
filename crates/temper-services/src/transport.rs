//! The transport layers every HTTP surface applies below its own root span.
//!
//! Companion to [`crate::cors`], and here for the same reason: temper-api and temper-mcp each
//! assemble their own router, so anything expressed twice is something one of them can be built
//! without. That is not hypothetical — `RequestDecompressionLayer` and the fallback handler were
//! both applied by `apply_transport_layers` (shared by the public and internal API apps) and by
//! neither of MCP's stacks, so on the agent-facing surface a gzip body reached the JSON extractor
//! still compressed, and an unmatched path was answered by the auth middleware rather than by a
//! 404 of any shape.
//!
//! The root span stays per-surface deliberately: temper-api names it `http_request` and temper-mcp
//! names it `mcp_request`, because three different things under one name is unreadable once they
//! are exported together.

use axum::Router;
use tower_http::decompression::RequestDecompressionLayer;

/// Apply the layers that sit **below** a surface's root span: the fallback handler and request
/// decompression.
///
/// Call this first, then add the surface's own root span and its CORS layer:
///
/// ```ignore
/// apply_base_layers(app)
///     .layer(axum::middleware::from_fn(root_span))
///     .layer(temper_services::cors::cors_layer(&config))
/// ```
///
/// Ordering matters and is preserved from the stack this was extracted from: decompression runs
/// innermost, so the body an extractor sees — and therefore any `DefaultBodyLimit` guarding it —
/// is measured in decompressed bytes.
pub fn apply_base_layers<S>(app: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    app.fallback(fallback_handler)
        .layer(RequestDecompressionLayer::new())
}

/// Answer an unmatched route with temper's structured error body.
///
/// Axum's default fallback returns a bare 404 with an empty body. Every other error a caller can
/// receive from temper is an [`crate::error::ErrorBody`], so a client that parses errors would
/// have to special-case exactly this one.
pub async fn fallback_handler(req: axum::extract::Request) -> axum::response::Response {
    use axum::response::IntoResponse;

    let path = req.uri().path().to_string();
    let method = req.method().to_string();
    tracing::warn!(path = %path, method = %method, "unmatched route");
    let body =
        crate::error::ErrorBody::new("NOT_FOUND", format!("No route matches {method} {path}"));
    (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response()
}
