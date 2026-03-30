use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::handlers;
use crate::middleware::auth;
use crate::openapi::ApiDoc;
use crate::state::AppState;

pub fn create_app(state: AppState) -> Router {
    use axum::routing::get;

    let public = Router::new()
        .route("/api/health", get(handlers::health::health_check))
        .route("/api/auth-login", get(handlers::auth_flow::login))
        .route("/api/auth-callback", get(handlers::auth_flow::callback));

    let protected = Router::new()
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
        .route(
            "/api/profile",
            get(handlers::profiles::get).patch(handlers::profiles::update),
        )
        .route(
            "/api/profile/auth-links",
            get(handlers::profiles::list_auth_links),
        )
        .route("/api/events", get(handlers::events::list))
        .route("/api/search", get(handlers::search::search))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let cors = cors_layer(&state);

    let mut app = Router::new().merge(public).merge(protected);

    if state.config.enable_swagger {
        app = app
            .merge(SwaggerUi::new("/api-docs/ui").url("/api-docs/openapi.json", ApiDoc::openapi()));
    }

    app.fallback(fallback_handler)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

async fn fallback_handler(req: axum::extract::Request) -> axum::response::Response {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();
    tracing::warn!(path = %path, method = %method, "unmatched route");
    axum::response::Response::builder()
        .status(404)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "error": {
                    "code": "NOT_FOUND",
                    "message": format!("No route matches {method} {path}")
                }
            })
            .to_string(),
        ))
        .expect("fallback response")
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
