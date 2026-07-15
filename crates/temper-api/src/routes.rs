use axum::Router;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::{DefaultOnFailure, TraceLayer};
use tracing::Span;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use crate::handlers;
use crate::middleware::auth;
use crate::openapi::ApiDoc;
use temper_services::state::AppState;

/// Unauthenticated routes. Documented.
fn public_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::health::health_check))
}

/// Authenticated but NOT system-access-gated — profile and self-service access
/// endpoints. Documented (a caller managing their own instance is a library
/// caller, not an operator).
fn auth_only_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::profiles::get, handlers::profiles::update))
        .routes(routes!(handlers::profiles::list_auth_links))
        .routes(routes!(handlers::access::create_request))
        .routes(routes!(
            handlers::access::get_own_request,
            handlers::access::withdraw_request
        ))
        .routes(routes!(handlers::access::get_settings))
        .routes(routes!(handlers::invitations::list_mine))
        .routes(routes!(handlers::invitations::accept))
        .routes(routes!(handlers::invitations::decline))
}

/// Authenticated AND system-access-gated — default-deny for all data routes.
/// Documented, except the operator-only `/api/access/admin/*` surface which is
/// mounted with plain `.route()` (no `#[utoipa::path]`) so it stays out of the
/// public contract.
fn gated_routes() -> OpenApiRouter<AppState> {
    use axum::routing::{get, patch, post};

    OpenApiRouter::new()
        .routes(routes!(
            handlers::resources::list,
            handlers::resources::create
        ))
        .routes(routes!(
            handlers::resources::get,
            handlers::resources::update,
            handlers::resources::delete
        ))
        .routes(routes!(handlers::resources::get_content))
        .routes(routes!(
            handlers::resources::provenance,
            handlers::resources::annotate
        ))
        .routes(routes!(handlers::reassign::reassign_resource))
        .routes(routes!(handlers::edges::list))
        .routes(routes!(handlers::edges::lineage))
        .routes(routes!(handlers::edges::assert))
        .routes(routes!(handlers::edges::retype))
        .routes(routes!(handlers::edges::reweight))
        .routes(routes!(handlers::edges::fold))
        .routes(routes!(handlers::facets::set_facet))
        .routes(routes!(handlers::graph::cogmap_neighborhood_slice))
        .routes(routes!(handlers::graph::region_composition))
        .routes(routes!(handlers::graph::context_panorama))
        .routes(routes!(handlers::graph::context_composition))
        .routes(routes!(handlers::graph::atlas_home))
        .routes(routes!(handlers::graph::cogmap_panorama))
        .routes(routes!(
            handlers::meta::get_meta,
            handlers::meta::update_meta
        ))
        .routes(routes!(
            handlers::resources::grant,
            handlers::resources::revoke
        ))
        .routes(routes!(
            handlers::contexts::list,
            handlers::contexts::create
        ))
        .routes(routes!(handlers::contexts::get))
        .routes(routes!(handlers::contexts::share_team))
        .routes(routes!(handlers::contexts::unshare_team))
        // Context orientation reads (T8) — the peers of the cognitive-map trio above.
        .routes(routes!(handlers::contexts::shape))
        .routes(routes!(handlers::contexts::region_metrics))
        .routes(routes!(handlers::contexts::materialize))
        .routes(routes!(handlers::teams::list, handlers::teams::create))
        .routes(routes!(handlers::teams::add_member))
        .routes(routes!(handlers::invitations::create))
        .routes(routes!(handlers::invitations::list))
        .routes(routes!(handlers::reassign::reassign_team))
        .routes(routes!(
            handlers::teams::detail,
            handlers::teams::update,
            handlers::teams::delete
        ))
        .routes(routes!(
            handlers::teams::remove_member,
            handlers::teams::change_role
        ))
        .routes(routes!(handlers::ingest::create))
        .routes(routes!(handlers::ingest::update))
        .routes(routes!(
            handlers::segments::list_blocks_handler,
            handlers::segments::append_block_handler
        ))
        .routes(routes!(handlers::segments::finalize_handler))
        .routes(routes!(handlers::cognitive_maps::genesis))
        .routes(routes!(handlers::cognitive_maps::reconcile))
        .routes(routes!(handlers::cognitive_maps::shape))
        .routes(routes!(handlers::cognitive_maps::materialize_delta))
        .routes(routes!(handlers::cognitive_maps::materialize))
        .routes(routes!(handlers::cognitive_maps::region_metrics))
        .routes(routes!(handlers::cognitive_maps::analytics))
        .routes(routes!(handlers::cognitive_maps::bind_team))
        .routes(routes!(handlers::cognitive_maps::unbind_team))
        .routes(routes!(
            handlers::cognitive_maps::grant,
            handlers::cognitive_maps::revoke
        ))
        .routes(routes!(
            handlers::invocations::open,
            handlers::invocations::list
        ))
        .routes(routes!(handlers::invocations::show))
        .routes(routes!(handlers::invocations::close))
        .routes(routes!(handlers::steward::delta))
        .routes(routes!(handlers::steward::advance))
        .routes(routes!(handlers::steward::sweep))
        .routes(routes!(handlers::steward::candidates))
        .routes(routes!(handlers::steward::dispatch))
        .routes(routes!(handlers::events::cursor))
        .routes(routes!(handlers::events::element_trail))
        .routes(routes!(handlers::search::search))
        // Operator-only re-embed trigger: enqueue embed jobs for chunks whose vector was produced by
        // a model that is no longer the one we embed with. The per-minute drain does the work; this is
        // only the trigger. Admin-gated on the caller's own identity, so an operator uses their normal
        // login rather than holding the drain's deploy secret.
        .route("/api/embed/admin/reembed", post(handlers::embed::reembed))
        // Operator-only access-gate admin surface — deliberately UNDOCUMENTED.
        // These handlers carry no `#[utoipa::path]`; plain `.route()` mounts them
        // without adding them to the OpenAPI contract.
        .route(
            "/api/access/admin/requests",
            get(handlers::access::list_pending),
        )
        .route(
            "/api/access/admin/requests/{id}",
            patch(handlers::access::review_request),
        )
        .route(
            "/api/access/admin/settings",
            get(handlers::access::get_admin_settings).patch(handlers::access::update_settings),
        )
        .route(
            "/api/access/admin/promote",
            post(handlers::access::promote_admin),
        )
        // Operator-only machine-principal registration (G3 Phase A). Mounted with plain
        // `.route()`, like `/api/access/admin/*` above, so it stays OUT of the OpenAPI
        // contract — it is an admin surface, not a public one. Its paths are allowlisted in
        // `.github/scripts/check-openapi-routes.sh`. The `is_system_admin` gate is enforced
        // inside each handler (load-bearing: the gated router admits everyone under
        // access_mode='open').
        .route(
            "/api/machine-clients",
            get(handlers::machine_clients::list).post(handlers::machine_clients::provision),
        )
        .route(
            "/api/machine-clients/{id}",
            get(handlers::machine_clients::get).delete(handlers::machine_clients::revoke),
        )
        .route(
            "/api/machine-clients/{id}/rebind",
            post(handlers::machine_clients::rebind),
        )
        .route(
            "/api/machine-clients/issue",
            post(handlers::machine_clients::issue),
        )
        .route(
            "/api/machine-clients/{id}/rotate-secret",
            post(handlers::machine_clients::rotate_secret),
        )
        // Operator-only connection provisioning (external systems as subscribed emitters, S1).
        // Same shape as machine-clients above and for the same reasons: plain `.route()`, out of
        // the OpenAPI contract, allowlisted in `.github/scripts/check-openapi-routes.sh`, and
        // gated inside the service (`machine_authz::authorize`, verbatim — a connection is a
        // machine principal wearing an integration's clothes).
        .route(
            "/api/connections",
            get(handlers::connections::list).post(handlers::connections::provision),
        )
        .route(
            "/api/connections/{id}",
            get(handlers::connections::get).delete(handlers::connections::revoke),
        )
        // The credential and the two capability tiers, each its own endpoint. They are separately
        // provisioned and both explicit — folding them into one PATCH would let a caller grant
        // reach while believing they were only registering a webhook.
        .route(
            "/api/connections/{id}/credential",
            post(handlers::connections::attach_credential),
        )
        .route(
            "/api/connections/{id}/webhook-events",
            post(handlers::connections::set_webhook_events),
        )
        .route(
            "/api/connections/{id}/tool-manifest",
            post(handlers::connections::set_tool_manifest),
        )
}

/// Internal, server-to-server only — gated by a shared secret, NOT `require_auth`.
/// Called by the co-deployed SAML Authorization Server before it mints a token.
/// Excluded from the OpenAPI contract entirely.
fn internal_routes() -> Router<AppState> {
    use axum::routing::post;

    Router::new().route(
        "/internal/saml/reconcile",
        post(handlers::internal_saml::reconcile),
    )
}

/// Internal cron-invoked embed endpoints — self-gated by EMBED_DISPATCH_SECRET (bearer),
/// NOT `require_auth`. Called by Vercel crons on a schedule; each handler checks the secret
/// itself (fail-closed when unset), so no auth-middleware layer is applied. Excluded from the
/// OpenAPI contract entirely.
///
/// - `/api/embed/dispatch` — the async-embed drain (issue #299).
/// - `/api/embed/warm` — cold-start warmup for server-side query embedding (issue #427).
///
/// NOTE: `embed::dispatch`'s `#[utoipa::path]` declares `get` only, but the route
/// mounts BOTH GET and POST on the same handler. This plain `.route()` (rather than
/// `routes!()`) is precisely why it can keep both methods AND stay out of the spec.
fn embed_internal_routes() -> Router<AppState> {
    use axum::routing::get;

    Router::new()
        .route(
            "/api/embed/dispatch",
            get(handlers::embed::dispatch).post(handlers::embed::dispatch),
        )
        // Cold-start warmup (issue #427): loads/exercises the ONNX embedder so a subsequent
        // server-side query embed on this instance is a cheap cached inference rather than a cold
        // model load that blows the query-embed budget. Same self-gated posture as `dispatch`.
        .route("/api/embed/warm", get(handlers::embed::warm))
}

pub fn create_app(state: AppState) -> Router {
    // Register documented sub-routers, then apply the same middleware layers as
    // before. `require_auth` is added last on the gated router so it is the
    // outermost layer (authenticate first, then check system access).
    let public = public_routes();
    let auth_only = auth_only_routes().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth::require_auth,
    ));
    let gated = gated_routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::system_access::require_system_access,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    // The documented sub-routers only contribute axum routes here; the OpenAPI
    // half is reconstructed DB-free by `openapi_spec()`.
    let (public, _) = public.split_for_parts();
    let (auth_only, _) = auth_only.split_for_parts();
    let (gated, _) = gated.split_for_parts();

    let internal = internal_routes().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::middleware::internal_auth::require_internal_signature,
    ));

    let embed_internal = embed_internal_routes();

    let cors = cors_layer(&state);

    let mut app = Router::new()
        .merge(public)
        .merge(auth_only)
        .merge(gated)
        .merge(internal)
        .merge(embed_internal);

    if state.config.enable_swagger {
        app =
            app.merge(SwaggerUi::new("/api-docs/ui").url("/api-docs/openapi.json", openapi_spec()));
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

/// The API contract, derived from the router. Pure: no `AppState`, no database,
/// no I/O. Seeded with `ApiDoc::openapi()` so info/tags/`SecurityAddon`/component
/// schemas survive, then merged with every documented sub-router. The internal
/// (`internal_routes`) and embed-drain (`embed_internal_routes`) surfaces are
/// deliberately NOT merged, so they never enter the spec.
pub fn openapi_spec() -> utoipa::openapi::OpenApi {
    use utoipa::Modify;

    let mut spec = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(public_routes())
        .merge(auth_only_routes())
        .merge(gated_routes())
        .split_for_parts()
        .1;

    // Applied here, not via `ApiDoc`'s `modifiers(...)`: those run against the seed spec, whose
    // `paths` map is empty until the merges above populate it.
    crate::openapi::SurfaceHeaderAddon.modify(&mut spec);
    spec
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
