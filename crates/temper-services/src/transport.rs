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

use axum::extract::DefaultBodyLimit;
use axum::Router;
use tower_http::decompression::RequestDecompressionLayer;

/// The largest request body either public HTTP surface will read, in **decompressed** bytes.
///
/// **This number is ratified, not derived, and that is the point of it being here.** Before this
/// constant existed the ceiling was axum's `DefaultBodyLimit` default — the same 2 MiB, but owned
/// by a dependency, so a framework upgrade could have moved temper's request ceiling with nothing
/// in this repository failing. Setting it explicitly changes no behaviour and moves the decision
/// here, where a change to it is a change someone made on purpose.
///
/// **Why 2 MiB is enough headroom, measured rather than assumed.** The largest first-party document
/// in this repository is ~70 KB; the largest resource body observed in the vault is ~34 KB. 2 MiB
/// is roughly thirty times the former.
///
/// **What that measurement does not cover, stated so a later decision to lower this has the
/// evidence it needs:** data artifacts and segmented ingest carry structured payloads and per-block
/// provenance, and neither was measured. A lower ceiling is defensible on document sizes alone and
/// is *not* defensible on these, which is why this ratifies the value in force instead of reducing
/// it.
///
/// Two limits deliberately sit outside this one, and both are narrower or wider on purpose:
/// temper-api's signed internal routes cap at 64 KiB, and its GitHub webhook route carries an
/// explicit 25 MiB ceiling because GitHub delivers up to that. A limit applied to a single route
/// runs inside this one and therefore wins for that route — which is what lets both coexist.
pub const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Apply the layers that sit **below** a surface's root span: the fallback handler, request
/// decompression, and the request-body ceiling.
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
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
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
