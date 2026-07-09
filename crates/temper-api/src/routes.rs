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
use temper_services::state::AppState;

pub fn create_app(state: AppState) -> Router {
    use axum::routing::{delete, get, post, put};

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
        .route(
            "/api/invitations/mine",
            get(handlers::invitations::list_mine),
        )
        .route(
            "/api/invitations/{token}/accept",
            post(handlers::invitations::accept),
        )
        .route(
            "/api/invitations/{token}/decline",
            post(handlers::invitations::decline),
        )
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
        .route(
            "/api/resources/{id}/provenance",
            get(handlers::resources::provenance),
        )
        .route(
            "/api/resources/{id}/reassign",
            post(handlers::reassign::reassign_resource),
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
        .route("/api/facets", post(handlers::facets::set_facet))
        .route(
            "/api/cogmaps/{id}/graph/slice",
            post(handlers::graph::cogmap_neighborhood_slice),
        )
        .route(
            "/api/graph/regions/composition",
            get(handlers::graph::region_composition),
        )
        .route(
            "/api/graph/contexts/panorama",
            get(handlers::graph::context_panorama),
        )
        .route(
            "/api/graph/contexts/composition",
            get(handlers::graph::context_composition),
        )
        .route("/api/graph/home", get(handlers::graph::atlas_home))
        .route(
            "/api/graph/cogmaps/{id}/panorama",
            get(handlers::graph::cogmap_panorama),
        )
        .route(
            "/api/resources/{id}/meta",
            get(handlers::meta::get_meta).put(handlers::meta::update_meta),
        )
        .route(
            "/api/resources/{id}/grants",
            post(handlers::resources::grant).delete(handlers::resources::revoke),
        )
        .route(
            "/api/contexts",
            get(handlers::contexts::list).post(handlers::contexts::create),
        )
        .route("/api/contexts/{id}", get(handlers::contexts::get))
        .route(
            "/api/contexts/{id}/teams",
            post(handlers::contexts::share_team),
        )
        .route(
            "/api/contexts/{id}/teams/{team_id}",
            delete(handlers::contexts::unshare_team),
        )
        .route(
            "/api/teams",
            get(handlers::teams::list).post(handlers::teams::create),
        )
        .route("/api/teams/{id}/members", post(handlers::teams::add_member))
        .route(
            "/api/teams/{id}/invite",
            post(handlers::invitations::create),
        )
        .route(
            "/api/teams/{id}/invitations",
            get(handlers::invitations::list),
        )
        .route(
            "/api/teams/{id}/reassign",
            post(handlers::reassign::reassign_team),
        )
        .route(
            "/api/teams/{id}",
            get(handlers::teams::detail)
                .patch(handlers::teams::update)
                .delete(handlers::teams::delete),
        )
        .route(
            "/api/teams/{id}/members/{profile_id}",
            delete(handlers::teams::remove_member).patch(handlers::teams::change_role),
        )
        .route("/api/ingest", post(handlers::ingest::create))
        .route("/api/ingest/{id}", put(handlers::ingest::update))
        .route(
            "/api/resources/{id}/blocks",
            get(handlers::segments::list_blocks_handler)
                .post(handlers::segments::append_block_handler),
        )
        .route(
            "/api/resources/{id}/finalize",
            post(handlers::segments::finalize_handler),
        )
        .route(
            "/api/cognitive-maps",
            post(handlers::cognitive_maps::genesis),
        )
        .route(
            "/api/cognitive-maps/{id}",
            put(handlers::cognitive_maps::reconcile),
        )
        .route(
            "/api/cognitive-maps/{id}/shape",
            get(handlers::cognitive_maps::shape),
        )
        .route(
            "/api/cognitive-maps/{id}/materialize-delta",
            get(handlers::cognitive_maps::materialize_delta),
        )
        .route(
            "/api/cognitive-maps/{id}/materialize",
            post(handlers::cognitive_maps::materialize),
        )
        .route(
            "/api/cognitive-maps/{id}/region-metrics",
            get(handlers::cognitive_maps::region_metrics),
        )
        .route(
            "/api/cognitive-maps/{id}/analytics",
            get(handlers::cognitive_maps::analytics),
        )
        .route(
            "/api/cognitive-maps/{id}/teams",
            post(handlers::cognitive_maps::bind_team),
        )
        .route(
            "/api/cognitive-maps/{id}/teams/{team_id}",
            delete(handlers::cognitive_maps::unbind_team),
        )
        .route(
            "/api/cognitive-maps/{id}/grants",
            post(handlers::cognitive_maps::grant).delete(handlers::cognitive_maps::revoke),
        )
        .route(
            "/api/invocations",
            post(handlers::invocations::open).get(handlers::invocations::list),
        )
        .route("/api/invocations/{id}", get(handlers::invocations::show))
        .route(
            "/api/invocations/{id}/close",
            post(handlers::invocations::close),
        )
        .route("/api/steward/{cogmap}/delta", get(handlers::steward::delta))
        .route(
            "/api/steward/{cogmap}/watermark",
            post(handlers::steward::advance),
        )
        .route("/api/steward/sweep", get(handlers::steward::sweep))
        .route(
            "/api/steward/candidates",
            get(handlers::steward::candidates),
        )
        .route("/api/steward/dispatch", post(handlers::steward::dispatch))
        .route(
            "/api/events/{kb_context_id}/cursor",
            get(handlers::events::cursor),
        )
        .route(
            "/api/graph/elements/{kind}/{id}/trail",
            get(handlers::events::element_trail),
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
        .route(
            "/api/access/admin/settings",
            get(handlers::access::get_admin_settings).patch(handlers::access::update_settings),
        )
        .route(
            "/api/access/admin/promote",
            post(handlers::access::promote_admin),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::system_access::require_system_access,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    // Internal, server-to-server only — gated by a shared secret, NOT `require_auth`.
    // Called by the co-deployed SAML Authorization Server before it mints a token.
    let internal = Router::new()
        .route(
            "/internal/saml/reconcile",
            post(handlers::internal_saml::reconcile),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_internal_signature,
        ));

    // Internal embed-dispatch drain (issue #299) — self-gated by EMBED_DISPATCH_SECRET (bearer),
    // NOT `require_auth`. Called by the Vercel cron on a schedule; the handler checks the secret
    // itself (fail-closed when unset), so no auth-middleware layer is applied.
    let embed_internal = Router::new().route(
        "/api/embed/dispatch",
        get(handlers::embed::dispatch).post(handlers::embed::dispatch),
    );

    let cors = cors_layer(&state);

    let mut app = Router::new()
        .merge(public)
        .merge(auth_only)
        .merge(gated)
        .merge(internal)
        .merge(embed_internal);

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
    let body = temper_services::error::ErrorBody::new(
        "NOT_FOUND",
        format!("No route matches {method} {path}"),
    );
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
