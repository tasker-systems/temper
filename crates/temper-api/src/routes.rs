use axum::Router;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::{DefaultOnFailure, TraceLayer};
use tracing::Span;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::handlers;
use crate::middleware::auth;
use crate::openapi::ApiDoc;
use crate::state::AppState;

pub fn create_app(state: AppState) -> Router {
    use axum::routing::{get, post, put};

    let public = Router::new().route("/api/health", get(handlers::health::health_check));

    // Authenticated but NOT gated by system access — profile and access endpoints.
    let auth_only = Router::new()
        .route(
            "/api/profile",
            get(handlers::profiles::get).patch(handlers::profiles::update),
        )
        .route(
            "/api/profile/auth-links",
            get(handlers::profiles::list_auth_links),
        )
        .route(
            "/api/access/requests",
            post(handlers::access::create_request),
        )
        .route(
            "/api/access/requests/me",
            get(handlers::access::get_own_request).delete(handlers::access::withdraw_request),
        )
        .route("/api/access/settings", get(handlers::access::get_settings))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    // Authenticated AND system-access-gated — default-deny for all data routes.
    let gated = Router::new()
        .route(
            "/api/resources",
            get(handlers::resources::list).post(handlers::resources::create),
        )
        .route(
            "/api/resources/{id}",
            get(handlers::resources::get)
                .patch(handlers::resources::update)
                .delete(handlers::resources::delete),
        )
        .route(
            "/api/resources/{id}/content",
            get(handlers::resources::get_content),
        )
        .route("/api/resources/{id}/edges", get(handlers::edges::list))
        .route("/api/relationships", post(handlers::edges::assert))
        .route(
            "/api/relationships/{edge_handle}/retype",
            post(handlers::edges::retype),
        )
        .route(
            "/api/relationships/{edge_handle}/reweight",
            post(handlers::edges::reweight),
        )
        .route(
            "/api/relationships/{edge_handle}/fold",
            post(handlers::edges::fold),
        )
        .route("/api/graph/subgraph", get(handlers::graph::get_subgraph))
        .route(
            "/api/resources/{id}/meta",
            get(handlers::meta::get_meta).put(handlers::meta::update_meta),
        )
        .route(
            "/api/contexts",
            get(handlers::contexts::list).post(handlers::contexts::create),
        )
        .route("/api/contexts/{id}", get(handlers::contexts::get))
        .route("/api/ingest", post(handlers::ingest::create))
        .route("/api/ingest/{id}", put(handlers::ingest::update))
        .route(
            "/api/cognitive-maps/{id}",
            put(handlers::cognitive_maps::reconcile),
        )
        .route(
            "/api/events/{kb_context_id}/cursor",
            get(handlers::events::cursor),
        )
        .route("/api/search", post(handlers::search::search))
        .route(
            "/api/access/admin/requests",
            get(handlers::access::list_pending),
        )
        .route(
            "/api/access/admin/requests/{id}",
            axum::routing::patch(handlers::access::review_request),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::system_access::require_system_access,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let cors = cors_layer(&state);

    let mut app = Router::new().merge(public).merge(auth_only).merge(gated);

    if state.config.enable_swagger {
        app = app
            .merge(SwaggerUi::new("/api-docs/ui").url("/api-docs/openapi.json", ApiDoc::openapi()));
    }

    app.fallback(fallback_handler)
        .layer(RequestDecompressionLayer::new())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path(),
                        version = ?request.version(),
                        profile_id = tracing::field::Empty,
                    )
                })
                .on_response(
                    |response: &axum::response::Response, latency: Duration, _span: &Span| {
                        tracing::info!(
                            status = response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "response",
                        );
                    },
                )
                .on_failure(DefaultOnFailure::new().level(tracing::Level::ERROR)),
        )
        .layer(cors)
        .with_state(state)
}

async fn fallback_handler(req: axum::extract::Request) -> axum::response::Response {
    use axum::response::IntoResponse;

    let path = req.uri().path().to_string();
    let method = req.method().to_string();
    tracing::warn!(path = %path, method = %method, "unmatched route");
    let body =
        crate::error::ErrorBody::new("NOT_FOUND", format!("No route matches {method} {path}"));
    (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response()
}

fn cors_layer(state: &AppState) -> CorsLayer {
    if state.config.cors_origins.is_empty() {
        // No origins configured — deny all cross-origin requests.
        // Use CORS_ORIGINS=* for permissive mode in development.
        CorsLayer::new()
    } else if state.config.cors_origins.len() == 1 && state.config.cors_origins[0] == "*" {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(
                state
                    .config
                    .cors_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect::<Vec<_>>(),
            )
            .allow_methods(Any)
            .allow_headers(Any)
    }
}
