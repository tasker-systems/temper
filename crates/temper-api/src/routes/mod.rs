//! The route table.
//!
//! Every route group temper-api serves is one row below: a group key (the postures
//! `.github/scripts/audit-route-auth.sh` names), the tier whose middleware stack wraps it, an
//! optional body limit, and the builders that mount it. The per-group route declarations live in
//! this module's sibling files; the layer policy for a tier is applied in exactly one place
//! (`apply_tier`); both app builders consume the same table, so the wiring cannot drift between
//! the single-process deploy and the Vercel internal function.
//!
//! Ordering is load-bearing and stated at the rows: axum applies `Router::layer` so the
//! LAST-added layer is the OUTERMOST (runs first on the request) — verified against the vendored
//! axum 0.8.9 source (`Endpoint::layer`/`PathRouter::layer` wrap the existing route). The
//! group doc comments carry the per-group lore.

mod admin;
mod auth_only;
mod blob_doors;
mod embed_internal;
mod gated;
mod internal;
mod public;
mod query;
mod slack_link_public;
mod webhook_intake;

pub use query::QUERY_MAX_BODY_BYTES;

use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::Router;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

use crate::middleware::{auth, internal_auth, relay_trust, system_access};
use crate::openapi::ApiDoc;
use admin::admin_routes;
use auth_only::auth_only_routes;
use blob_doors::{blob_commit_body_limit, blob_commit_routes, blob_segment_routes};
use embed_internal::embed_internal_routes;
use gated::gated_routes;
use internal::{internal_routes, slack_link_internal_routes, slack_mint_internal_routes};
use public::public_routes;
use slack_link_public::slack_link_public_routes;
use temper_services::state::AppState;
use webhook_intake::webhook_intake_routes;

/// Which middleware stack a group's router is wrapped in. The stack itself is applied in
/// exactly one place — [`apply_tier`]'s match arm for the tier — so a tier's policy exists
/// once, and a group's tier is data a script can assert.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tier {
    /// No middleware: by-design public (`/health`).
    Public,
    /// `require_auth` — authenticated, not system-access-gated.
    AuthOnly,
    /// The gated chain: system access, auth, the relay-trust carrier, and the inherited
    /// body ceiling. See [`GATED_MAX_BODY_BYTES`] and the row comments for the order.
    Gated,
    /// Rate limit inside a shared-secret HMAC signature gate; the kind names WHICH
    /// signature — three groups, three secrets, one scheme.
    InternalHmac(SignatureKind),
    /// No middleware: the gate is inside the handler (a self-checked secret, a JWKS
    /// attestation, or a PKCE callback).
    SelfGated,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SignatureKind {
    /// `require_internal_signature` (`INTERNAL_RECONCILE_SECRET`) — the SAML AS pair.
    Reconcile,
    /// `require_slack_link_signature` (`SLACK_LINK_SECRET`) — link-state.
    SlackLink,
    /// `require_slack_mint_signature` (`SLACK_MINT_SECRET`) — act-as-the-human mint.
    SlackMint,
}

/// A group's route declarations. Documented groups ride `OpenApiRouter` and contribute
/// to the published contract; undocumented groups are plain axum routers and never
/// enter the spec.
enum GroupRoutes {
    Documented(fn() -> OpenApiRouter<AppState>),
    Undocumented(fn() -> Router<AppState>),
}

/// An optional per-group `DefaultBodyLimit`. `CommitDoor` is derived from the config
/// per app-build (the D7 threshold plus multipart overhead) rather than a constant.
enum BodyLimit {
    Fixed(usize),
    CommitDoor,
}

/// Which app builders mount the group.
enum Serves {
    /// `create_app` only — the single-process deploy (local dev, e2e, self-hosted).
    AppOnly,
    /// Both builders — the group is also served by `create_internal_app` (the separate
    /// Vercel function with the longer `maxDuration`).
    BothBuilders,
}

struct Group {
    /// The audit scripts' group key — unchanged from the pre-table sub-router names.
    /// Read textually by the scripts (they assert these rows), never by the compiler.
    #[allow(dead_code)]
    key: &'static str,
    tier: Tier,
    build: GroupRoutes,
    body_limit: Option<BodyLimit>,
    serves: Serves,
}

/// The table. One row per group; the ordering below is the merge order in
/// [`create_app`] (rows here that only [`create_internal_app`] mounts carry
/// `Serves::BothBuilders` and are mounted in this order there too). The rationale a
/// reviewer needs is at the row; the per-group lore is in the sibling files' docs.
/// The skip is load-bearing: the audit scripts assert these rows TEXTUALLY
/// (`key: "<group>", tier: <spelling>` must stay one greppable line), so rustfmt
/// is not free to re-flow the table into multi-line struct literals.
#[rustfmt::skip]
fn route_table() -> Vec<Group> {
    use GroupRoutes::*;
    vec![
        // By-design public: a health check discloses nothing and authenticates nobody.
        Group { key: "public_routes", tier: Tier::Public, build: Documented(public_routes), body_limit: None, serves: Serves::AppOnly },
        // Self-service: authenticated, but no system-access gate — a caller managing
        // their own instance is a library caller, not an operator.
        Group { key: "auth_only_routes", tier: Tier::AuthOnly, build: Documented(auth_only_routes), body_limit: None, serves: Serves::AppOnly },
        // Default-deny for all data routes. Addition order (INNER → OUTER, so execution
        // runs outermost-first): require_system_access, require_auth, relay_trust,
        // DefaultBodyLimit. Relay-trust runs BEFORE the auth layers — it rejects nothing
        // and only plants the `RelayedSurface` extension beside a valid service
        // credential; authorization stays with `require_auth` on the caller's own
        // bearer. (The pre-table comment called this position INNERMOST — a mis-statement
        // of axum's ordering, corrected here with the source cited in the module doc.)
        // The inherited 25 MB ceiling is the OUTER layer so the doors that chose their
        // own limits (the blob rows below, `/api/query` inside the group) stay inner
        // and win on their routes — the network door's ruling 3, design §D4.
        Group { key: "gated_routes", tier: Tier::Gated, build: Documented(gated_routes), body_limit: None, serves: Serves::AppOnly },
        // The system-admin surface: the gated tier unchanged, its own row so the operator
        // surface is one auditable set. The tier admits any approved principal; the gate that
        // makes these routes admin-only is the `&SystemAdmin` proof each service requires,
        // minted in the handler before dispatch (see `admin.rs` for the membership rule).
        Group { key: "admin_routes", tier: Tier::Gated, build: Documented(admin_routes), body_limit: None, serves: Serves::AppOnly },
        // The two blob doors, at the gated tier with their own body limits INNER to the
        // tier stack so the decisions they chose win on their routes. The commit door's
        // bound is the config's D7 threshold plus multipart overhead — the transport
        // never fires first over a body the handler would legally accept.
        Group { key: "blob_commit_routes", tier: Tier::Gated, build: Documented(blob_commit_routes), body_limit: Some(BodyLimit::CommitDoor), serves: Serves::AppOnly },
        // The segment door's bound is the platform ceiling the plan numbers pin
        // (4.5 MB), not the config — the staging ceiling is enforced with its own
        // vocabulary in the service.
        Group { key: "blob_segment_routes", tier: Tier::Gated, build: Documented(blob_segment_routes), body_limit: Some(BodyLimit::Fixed(blob_doors::BLOB_SEGMENT_MAX_BODY_BYTES)), serves: Serves::AppOnly },
        // The three internal HMAC groups — one scheme, three secrets, three routers.
        // Signature gates mount in BOTH builders; losing one mount is not a downgrade
        // to authenticated-but-broad, it is the group served ungated on that surface.
        // The rate-limit layer rides the RECONCILE pair only (the base wiring applied
        // it there and nowhere else) and sits INNER to the signature so an unsigned
        // caller gets the 401 and never spends the signed caller's budget.
        Group { key: "internal_routes", tier: Tier::InternalHmac(SignatureKind::Reconcile), build: Undocumented(internal_routes), body_limit: None, serves: Serves::BothBuilders },
        Group { key: "slack_link_internal_routes", tier: Tier::InternalHmac(SignatureKind::SlackLink), build: Undocumented(slack_link_internal_routes), body_limit: None, serves: Serves::BothBuilders },
        Group { key: "slack_mint_internal_routes", tier: Tier::InternalHmac(SignatureKind::SlackMint), build: Undocumented(slack_mint_internal_routes), body_limit: None, serves: Serves::BothBuilders },
        // The IdP's PKCE redirect target — carries no bearer and no signature by
        // design; its authentication is the code exchange plus the burned state nonce.
        Group { key: "slack_link_public_routes", tier: Tier::SelfGated, build: Undocumented(slack_link_public_routes), body_limit: None, serves: Serves::AppOnly },
        // Vercel crons — each handler checks EMBED_DISPATCH_SECRET itself, fail-closed
        // when unset. Served by both builders so the internal function keeps its
        // longer maxDuration for the same paths.
        Group { key: "embed_internal_routes", tier: Tier::SelfGated, build: Undocumented(embed_internal_routes), body_limit: None, serves: Serves::BothBuilders },
        // Vercel Connect's webhook intake — self-gated on the broker's RS256
        // attestation (a third party's signature against a remote JWKS, not a shared
        // secret), and create_app only: Connect forwards to the public host. The row's
        // body_limit is None because the door sets its own 25 MiB
        // (GITHUB_MAX_WEBHOOK_BYTES) INSIDE the group fn — the table column is not the
        // whole truth for this one door; the limit is not config-derivable at the table.
        Group { key: "webhook_intake_routes", tier: Tier::SelfGated, build: Undocumented(webhook_intake_routes), body_limit: None, serves: Serves::AppOnly },
    ]
}

/// Build one group's axum router and wrap it in its tier's stack. The tier match below
/// is the ONLY place a middleware stack is spelled for a tier; the rows above are the
/// only place a group is bound to one.
fn mount_group(group: &Group, state: &AppState) -> Router<AppState> {
    let router = match group.build {
        GroupRoutes::Documented(build) => build().split_for_parts().0,
        GroupRoutes::Undocumented(build) => build(),
    };
    // The group's own body limit first, so it is INNER to the tier stack and its
    // decision wins on the group's routes (the innermost `DefaultBodyLimit` is the one
    // the extractor sees).
    let router = match group.body_limit {
        Some(BodyLimit::Fixed(n)) => router.layer(DefaultBodyLimit::max(n)),
        Some(BodyLimit::CommitDoor) => {
            router.layer(DefaultBodyLimit::max(blob_commit_body_limit(state)))
        }
        None => router,
    };
    apply_tier(router, group.tier, state)
}

/// The per-tier middleware stacks — one arm per tier, applied nowhere else. Addition
/// order here is INNER first (axum's last-added layer is the outermost, per the module
/// doc), so read each arm bottom-up for the request's execution order.
fn apply_tier(router: Router<AppState>, tier: Tier, state: &AppState) -> Router<AppState> {
    match tier {
        Tier::Public | Tier::SelfGated => router,
        Tier::AuthOnly => router.layer(from_fn_with_state(state.clone(), auth::require_auth)),
        Tier::Gated => router
            .layer(from_fn_with_state(
                state.clone(),
                system_access::require_system_access,
            ))
            .layer(from_fn_with_state(state.clone(), auth::require_auth))
            // INNERMOST among the auth middlewares it rides with, OUTER to them in
            // fact: relay-trust rejects nothing, plants the `RelayedSurface` extension
            // beside a valid service credential (the network door's attribution
            // carrier — see `middleware::relay_trust`), and authorization stays with
            // `require_auth` on the caller's bearer.
            .layer(from_fn_with_state(
                state.clone(),
                relay_trust::require_relay_trust,
            ))
            // Ruling 3: the inherited default stops being an invisible second gate —
            // the doors that chose their own limits stay inner and win.
            .layer(DefaultBodyLimit::max(GATED_MAX_BODY_BYTES)),
        Tier::InternalHmac(kind) => {
            // The rate-limit layer rides ONLY the reconcile pair's router — the base wiring
            // applied it there and to nothing else (the slack groups carry signature only).
            // Extending it to the slack groups is a declared behavioral change, not a
            // refactor; do not fold it in here silently.
            let router = match kind {
                SignatureKind::Reconcile => router.layer(from_fn_with_state(
                    state.clone(),
                    temper_services::rate_limit::require_route_rate_limit,
                )),
                SignatureKind::SlackLink | SignatureKind::SlackMint => router,
            };
            match kind {
                SignatureKind::Reconcile => router.layer(from_fn_with_state(
                    state.clone(),
                    internal_auth::require_internal_signature,
                )),
                SignatureKind::SlackLink => router.layer(from_fn_with_state(
                    state.clone(),
                    internal_auth::require_slack_link_signature,
                )),
                SignatureKind::SlackMint => router.layer(from_fn_with_state(
                    state.clone(),
                    internal_auth::require_slack_mint_signature,
                )),
            }
        }
    }
}

/// The body ceiling the gated router inherits (the network door's ruling 3, design §D4).
///
/// Axum's 2 MiB default was this router's operative limit on every route that never chose
/// one of its own — an invisible second gate the MCP relay's callers could not see: the MCP
/// edge accepts 25 MB (`MCP_MAX_BODY_BYTES`, temper-mcp's router), so through the network
/// door a legal create died 413 at the API — work the direct binding performed. Raised to
/// the edge's contract so the 25 MB ceiling stays the ONE user-visible limit on the tool
/// surface and the API's per-route defaults stop being a second, silent one. 25 MB matches
/// `GITHUB_MAX_WEBHOOK_BYTES`, the repo's existing generous transport bound, which puts the
/// number on an in-repo precedent rather than on a guess.
///
/// The doors that DID choose — `/api/query`'s composition backstop (`QUERY_MAX_BODY_BYTES`,
/// also 25 MB, declared separately because it is held against the largest legal composition),
/// the blob segment door's platform-derived 4.5 MB, the commit door's config-derived threshold —
/// merge with their own `DefaultBodyLimit` layers INNER to this one, so their decisions win on
/// their routes and this number changes nothing there.
const GATED_MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

pub fn create_app(state: AppState) -> Router {
    // Mount every row in table order. The documented sub-routers only contribute axum
    // routes here; the OpenAPI half is reconstructed DB-free by `openapi_spec()`.
    let mut app = Router::new();
    for group in route_table() {
        app = app.merge(mount_group(&group, &state));
    }

    if state.config.enable_swagger {
        // Swagger's own bundle is scripts, styles and images from this origin, all of which the
        // app-wide `default-src 'none'` forbids — so the explorer would load as a blank page under
        // the baseline. Its policy is set here, on the only routes it covers, and is still
        // origin-locked: nothing third-party, no framing, no `<base>` rewrite. The `if_not_present`
        // baseline then leaves it alone.
        //
        // This is the *developer* explorer, reached only when `ENABLE_SWAGGER` is set. That is why
        // a looser policy is acceptable here and would not be as a shared default.
        const SWAGGER_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'              'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:;              frame-ancestors 'none'; base-uri 'none'";

        let swagger: Router<AppState> = SwaggerUi::new("/api-docs/ui")
            .url("/api-docs/openapi.json", openapi_spec())
            .into();
        app = app.merge(
            temper_services::transport::override_content_security_policy(
                swagger,
                SWAGGER_CONTENT_SECURITY_POLICY,
            ),
        );
    }

    apply_transport_layers(app, state)
}

/// The internal/system-only app — the groups the table marks `Serves::BothBuilders`
/// (the signature-gated pairs and the self-gated crons) with the same transport layers
/// as [`create_app`], but none of the public/auth/gated API and no Swagger.
///
/// This exists so Vercel can serve these paths from a **separate function**
/// (`api/internal.rs`) with its own `maxDuration`. The embed crons run ONNX
/// warmups and drain passes that can exceed the 60s public-API ceiling; isolating
/// them here lets that ceiling be raised for the crons without letting a public
/// request hang for the same window. `create_app` still mounts these routes too,
/// so a single-process deploy (local dev, e2e, self-hosted) keeps serving the full
/// surface from one binary — the split matters only for Vercel's per-function
/// timeout model.
pub fn create_internal_app(state: AppState) -> Router {
    // The same table, the same tier stacks — the internal function must carry the seam
    // identically to the public one, or the second serving path would be the unlimited
    // copy of the first.
    let mut app = Router::new();
    for group in route_table() {
        if matches!(group.serves, Serves::BothBuilders) {
            app = app.merge(mount_group(&group, &state));
        }
    }

    apply_transport_layers(app, state)
}

/// Apply the shared transport-layer stack (fallback, request decompression, HTTP
/// tracing, CORS) and bind `state`. Shared by [`create_app`] and
/// [`create_internal_app`] so both surfaces observe and trace requests identically.
fn apply_transport_layers(app: Router<AppState>, state: AppState) -> Router {
    let cors = temper_services::cors::cors_layer(&state.config.cors_origins);

    temper_services::transport::apply_base_layers(app)
        .layer(axum::middleware::from_fn(root_span))
        .layer(cors)
        .with_state(state)
}

/// The `http_request` root span, and the end of its life.
///
/// Replaced `tower_http`'s `TraceLayer` when the exporter landed: `TraceLayer` clones its span into
/// the response body, so the span outlives every middleware and no flush can ever see it. See
/// `temper_telemetry::request_span` for the measurement behind that. The span name, the field set,
/// and the `response` event are unchanged — this is a change of mechanism, not of the convention in
/// `internal/development/span-field-conventions.md`.
async fn root_span(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    temper_telemetry::traced_request(request, next, |request| {
        temper_telemetry::root_span!("http_request", request)
    })
    .await
}

/// The API contract, derived from the router. Pure: no `AppState`, no database,
/// no I/O. Seeded with `ApiDoc::openapi()` so info/tags/`SecurityAddon`/component
/// schemas survive, then merged with every DOCUMENTED group in the table. The
/// undocumented groups (`internal_routes`, `slack_link_internal_routes`,
/// `slack_mint_internal_routes`, `slack_link_public_routes`, `embed_internal_routes`,
/// `webhook_intake_routes`) are deliberately never merged, so they never enter the
/// spec.
pub fn openapi_spec() -> utoipa::openapi::OpenApi {
    use utoipa::Modify;

    let mut spec = OpenApiRouter::with_openapi(ApiDoc::openapi());
    for group in route_table() {
        if let GroupRoutes::Documented(build) = group.build {
            spec = spec.merge(build());
        }
    }
    let mut spec = spec.split_for_parts().1;

    // Applied here, not via `ApiDoc`'s `modifiers(...)`: those run against the seed spec, whose
    // `paths` map is empty until the merges above populate it.
    crate::openapi::SurfaceHeaderAddon.modify(&mut spec);
    // Same reason, for the component half: an open enum reached only through a route's
    // request/response body is not in `components` until the merges above collect it, so a modifier
    // registered on `ApiDoc` would run before the schema it repairs exists.
    crate::openapi::OpenStringEnumAddon.modify(&mut spec);
    crate::openapi::ApidogFolderAddon.modify(&mut spec);
    spec
}
