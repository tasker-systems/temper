use axum::Router;
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
        .routes(routes!(handlers::access::create_review_request))
        .routes(routes!(handlers::access::get_settings))
        .routes(routes!(handlers::invitations::list_mine))
        .routes(routes!(handlers::invitations::count_mine))
        .routes(routes!(handlers::invitations::accept))
        .routes(routes!(handlers::invitations::decline))
        .routes(routes!(handlers::slack_disconnect::disconnect_me))
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
        .routes(routes!(handlers::data_artifacts::list))
        .routes(routes!(handlers::data_artifacts::get))
        .routes(routes!(handlers::data_artifacts::commit))
        .routes(routes!(handlers::blobs::commit))
        .routes(routes!(handlers::blobs::get))
        // One `.routes()` per handler: the multi-handler form is for same-path method
        // grouping (the ingest blocks GET+POST shape); distinct paths in one call mangle
        // the mounted patterns into overlaps.
        .routes(routes!(handlers::blobs::begin_upload))
        .routes(routes!(handlers::blobs::upload_progress))
        .routes(routes!(handlers::blobs::finalize_upload))
        .merge(blob_segment_routes())
        .routes(routes!(handlers::data_artifact_shapes::list_shapes))
        .routes(routes!(handlers::data_artifact_shapes::get_shape))
        .routes(routes!(handlers::data_artifact_shapes::declare_shape))
        .routes(routes!(
            handlers::resources::provenance,
            handlers::resources::annotate
        ))
        .routes(routes!(handlers::reassign::reassign_resource))
        .routes(routes!(handlers::edges::list))
        .routes(routes!(handlers::evidence::evidence))
        // Both methods on `/api/resources/{id}/citation-audits` — one `routes!` group, as with the
        // resource CRUD trio above, so the path is declared once.
        .routes(routes!(
            handlers::citation_audits::record,
            handlers::citation_audits::list
        ))
        .routes(routes!(handlers::edges::lineage))
        .routes(routes!(handlers::edges::assert))
        .routes(routes!(handlers::edges::retype))
        .routes(routes!(handlers::edges::reweight))
        .routes(routes!(handlers::edges::fold))
        .routes(routes!(handlers::facets::set_facet))
        .routes(routes!(handlers::facets::list_resource_facets))
        .routes(routes!(
            handlers::facets::set_edge_facet,
            handlers::facets::list_edge_facets
        ))
        .routes(routes!(handlers::graph::cogmap_neighborhood_slice))
        .routes(routes!(handlers::graph::region_composition))
        .routes(routes!(handlers::graph::context_panorama))
        .routes(routes!(handlers::graph::context_composition))
        .routes(routes!(handlers::graph::entry))
        .routes(routes!(handlers::graph::traverse))
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
        .routes(routes!(handlers::contexts::get, handlers::contexts::delete))
        .routes(routes!(handlers::contexts::restore))
        .routes(routes!(handlers::contexts::share_team))
        .routes(routes!(handlers::contexts::unshare_team))
        .routes(routes!(handlers::contexts::reassign))
        .routes(routes!(handlers::contexts::rename))
        // Context orientation reads (T8) — the peers of the five cognitive-map orientation reads
        // below (shape, materialize-delta, materialize, region-metrics, analytics).
        .routes(routes!(handlers::contexts::shape))
        .routes(routes!(handlers::contexts::region_metrics))
        .routes(routes!(handlers::contexts::materialize_delta))
        .routes(routes!(handlers::contexts::materialize))
        .routes(routes!(handlers::contexts::analytics))
        .routes(routes!(handlers::teams::list, handlers::teams::create))
        .routes(routes!(handlers::teams::add_member))
        .routes(routes!(handlers::invitations::create))
        .routes(routes!(handlers::invitations::list))
        .routes(routes!(handlers::invitations::revoke))
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
        .routes(routes!(
            handlers::cognitive_maps::genesis,
            handlers::cognitive_maps::list
        ))
        .routes(routes!(
            handlers::cognitive_maps::reconcile,
            handlers::cognitive_maps::show
        ))
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
        .routes(routes!(handlers::auditor::sweep))
        .routes(routes!(handlers::auditor::dispatch))
        .routes(routes!(handlers::auditor::complete))
        .routes(routes!(handlers::events::cursor))
        .routes(routes!(handlers::events::element_trail))
        // The vocabularies each kind of work carries. Caller-independent answers over the
        // embedded schemas — still on the gated surface, because caller-independence is a
        // property of the answer and not a reason to publish it.
        .routes(routes!(handlers::schema::list_doc_types))
        .routes(routes!(handlers::schema::describe_doc_type))
        .routes(routes!(handlers::schema::describe_open_meta))
        .routes(routes!(handlers::search::search))
        .merge(query_routes())
        .routes(routes!(handlers::slack_disconnect::admin_disconnect))
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
        // The counting siblings of the two queue reads. Static `/count` beats the `{id}`
        // pattern in the router, and they are GET while `{id}` is PATCH, so neither shadows
        // the other. Same undocumented posture as the lists they count.
        .route(
            "/api/access/admin/requests/count",
            get(handlers::access::count_pending),
        )
        .route(
            "/api/access/admin/requests/{id}",
            patch(handlers::access::review_request),
        )
        // Same posture, same reason: the D15 reconsideration inbox is read and closed by an
        // operator, never by a library caller administering their own access.
        .route(
            "/api/access/admin/reviews",
            get(handlers::access::list_reviews),
        )
        .route(
            "/api/access/admin/reviews/count",
            get(handlers::access::count_reviews),
        )
        .route(
            "/api/access/admin/reviews/{id}",
            patch(handlers::access::close_review),
        )
        .route(
            "/api/access/admin/settings",
            get(handlers::access::get_admin_settings).patch(handlers::access::update_settings),
        )
        .route(
            "/api/access/admin/promote",
            post(handlers::access::promote_admin),
        )
        .route(
            "/api/access/admin/demote",
            post(handlers::access::demote_admin),
        )
        // The admin standing acts (Task 13). Same operator-only convention as their neighbours:
        // plain `.route()`, out of the OpenAPI contract, allowlisted in
        // `.github/scripts/check-openapi-routes.sh`. The admin gate is in each handler.
        .route(
            "/api/access/admin/principals/{id}/approve",
            post(handlers::access::approve_principal),
        )
        .route(
            "/api/access/admin/principals/{id}/revoke",
            post(handlers::access::revoke_principal),
        )
        .route(
            "/api/access/admin/principals/{id}/deactivate",
            post(handlers::access::deactivate_principal),
        )
        .route(
            "/api/access/admin/principals/{id}/reactivate",
            post(handlers::access::reactivate_principal),
        )
        // The admin ledger's read surface — operator-only, so plain `.route()` and OUT of the
        // OpenAPI contract like its neighbours above. Authorization is in
        // `admin_ledger_service`, which gates per act family rather than with a prelude, and
        // denies with 404 so a refusal discloses nothing about the subject.
        .route("/api/admin/ledger", get(handlers::admin_ledger::list))
        // Machine-principal registration (G3 Phase A). Mounted with plain `.route()`, like
        // `/api/access/admin/*` above, so it stays OUT of the OpenAPI contract. Its paths are
        // allowlisted in `.github/scripts/check-openapi-routes.sh`.
        //
        // NOT admin-only, despite sitting among the admin mounts. The gate is
        // `is_system_admin OR owner of the machine's owning team` (`machine_authz::authorize`),
        // so any authenticated profile that owns any team can reach `provision`, `issue`, and
        // `apply_reach`. Only `rebind` is admin-only (`machine_registration_service::rebind`).
        //
        // The gate lives in the SERVICES, not in these handlers — the handlers are gate-free by
        // design, as `handlers::machine_clients`' module doc explains. Treat it as load-bearing,
        // not defense-in-depth: how much the router's `require_system_access` layer actually
        // excludes is an operational setting an instance can change at any time, so the service
        // check is the only guarantee that does not move. Do not relax it on the strength of a
        // configuration value read at some past moment.
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
        // A team's read-reach on the connection, its own endpoint (a `kb_access_grants` write, not
        // a connection-row mutation). Owning ≠ reaching, so this is separate from provisioning.
        // Grant and revoke share the path — POST adds, DELETE removes — both carrying the team.
        .route(
            "/api/connections/{id}/reach",
            post(handlers::connections::grant_reach).delete(handlers::connections::revoke_reach),
        )
        // Operator-only subscription management (external systems as subscribed emitters, S2).
        // Same shape as connections above: plain .route(), out of the OpenAPI contract, gated
        // inside the service (require_manage_on_team + kb_access_grants reach-grant read).
        .route(
            "/api/subscriptions",
            get(handlers::subscriptions::list).post(handlers::subscriptions::create),
        )
        .route(
            "/api/subscriptions/{id}",
            get(handlers::subscriptions::get).delete(handlers::subscriptions::revoke),
        )
}

/// Internal, server-to-server only — gated by a shared secret, NOT `require_auth`.
/// Called by the co-deployed SAML Authorization Server before it mints a token.
/// Excluded from the OpenAPI contract entirely.
fn internal_routes() -> Router<AppState> {
    use axum::routing::post;

    Router::new()
        .route(
            "/internal/saml/reconcile",
            post(handlers::internal_saml::reconcile),
        )
        // Same caller and the SAME key as its neighbour, which is why it belongs on this router
        // rather than one of its own: the AS asks who a `sub` resolves to so it can record an owner
        // on the refresh chain it is about to mint.
        .route(
            "/internal/principal/resolve",
            post(handlers::internal_saml::resolve_principal),
        )
}

/// Internal, server-to-server only — gated by `require_slack_link_signature`, NOT
/// `require_auth`. Called by the Slack mention agent on every mention to ask what to say to
/// the mentioning user: already linked, or here is a fresh authorize URL.
///
/// A router of its own rather than a route on [`internal_routes`] because the two carry
/// different keys: `internal_routes` is layered with `require_internal_signature`
/// (`INTERNAL_RECONCILE_SECRET`), and gating this route on the reconcile secret would let
/// either principal forge the other's calls. One scheme, two secrets, two routers — the
/// layer is applied at each merge site so the route can never be mounted ungated.
/// Excluded from the OpenAPI contract entirely.
fn slack_link_internal_routes() -> Router<AppState> {
    use axum::routing::post;

    Router::new().route(
        "/internal/slack/link-state",
        post(handlers::slack_link::slack_link_state),
    )
}

/// Internal, server-to-server only — gated by `require_slack_mint_signature`, NOT `require_auth`
/// and NOT the link-state gate. Called by the Slack mention agent to obtain an
/// act-as-the-human access token for a mentioning user.
///
/// A **third** router rather than a second route on [`slack_link_internal_routes`], even though
/// the caller is the same agent, because the keys must differ. Link-state answers a question
/// ("is this principal linked?"); this vends a credential carrying that human's entire reach.
/// Sharing one key would make compromise of the cheap capability yield the expensive one — the
/// same reasoning that already separates `internal_routes` from `slack_link_internal_routes`,
/// applied where the stakes are highest. One scheme, three secrets, three routers — the layer is
/// applied at each merge site so the route can never be mounted ungated.
/// Excluded from the OpenAPI contract entirely.
fn slack_mint_internal_routes() -> Router<AppState> {
    use axum::routing::post;

    Router::new().route(
        "/internal/slack/mint",
        post(handlers::slack_mint::slack_mint),
    )
}

/// The browser-facing Slack link callback — the registered `redirect_uri`.
///
/// Ungated by design: it is the IdP's redirect target, so it carries no bearer and no
/// signature. Its authentication is the PKCE code exchange plus the single-use state nonce
/// it burns, and it renders HTML rather than JSON because a human is looking at it.
/// `create_app` only — the internal function never serves a browser. Excluded from the
/// OpenAPI contract entirely.
fn slack_link_public_routes() -> Router<AppState> {
    use axum::routing::get;

    Router::new().route(
        "/api/auth/slack/callback",
        get(handlers::slack_link::callback),
    )
}

/// Internal cron-invoked embed (and Slack intents reaper) endpoints — self-gated by
/// EMBED_DISPATCH_SECRET (bearer), NOT `require_auth`. Called by Vercel crons on a schedule; each
/// handler checks the secret itself (fail-closed when unset), so no auth-middleware layer is
/// applied. Excluded from the OpenAPI contract entirely.
///
/// - `/api/embed/dispatch` — the async-embed drain (issue #299).
/// - `/api/embed/warm` — cold-start warmup for server-side query embedding (issue #427).
/// - `/api/slack/intents/reap` — hourly retention sweep for expired/consumed link intents (T4).
/// - `/api/as/reap` — daily retention sweep for the three Authorization Server tables (TMPR-56).
///
/// NOTE: `embed::dispatch`'s `#[utoipa::path]` declares `get` only, but the route
/// mounts BOTH GET and POST on the same handler. This plain `.route()` (rather than
/// `routes!()`) is precisely why it can keep both methods AND stay out of the spec.
/// `slack_disconnect::reap_intents` carries no `#[utoipa::path]` at all, for the same reason.
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
        // Hourly retention sweep for Slack link intents (T4/Task 8). Same self-gated posture as
        // `dispatch`/`warm` (`require_dispatch_secret`, reusing EMBED_DISPATCH_SECRET) and the same
        // GET+POST-on-one-handler shape.
        .route(
            "/api/slack/intents/reap",
            get(handlers::slack_disconnect::reap_intents)
                .post(handlers::slack_disconnect::reap_intents),
        )
        // Daily retention sweep for the AS tables (TMPR-56) — kb_saml_replay, kb_oauth_flow and
        // kb_oauth_refresh_tokens, none of which anything had ever deleted from. Same self-gated
        // posture and GET+POST-on-one-handler shape as `dispatch`/`warm`. It belongs in this group
        // rather than on the public function for the reason the group exists: the FIRST run drains
        // a backlog accumulated since 20260701000006, and a capped pass over months of rows is not
        // work to put behind the 60s public ceiling.
        .route(
            "/api/as/reap",
            get(handlers::as_reap::reap_as_tables).post(handlers::as_reap::reap_as_tables),
        )
        // Reconcile-channel health check (goal 01a035eb, clause
        // a-de-provisioning-that-did-not-happen-is-visible-to-an-operator). Reads the fact
        // temper-cloud records when a fail-open internal call does not reach us and turns it into a
        // signal. It belongs in this group for the group's posture rather than its duration — the
        // check is one indexed read — and specifically because `require_dispatch_secret` is already
        // set on every deployment that runs the other crons, so this one cannot go dark for want of
        // a variable nobody knew to set. Same GET+POST-on-one-handler shape as `dispatch`/`warm`.
        .route(
            "/api/internal-calls/health",
            get(handlers::internal_call_health::check_internal_calls)
                .post(handlers::internal_call_health::check_internal_calls),
        )
        // Region-clock drain (goal 019fc46c): runs T6's two clocks off the request path. Same
        // self-gated posture and GET+POST-on-one-handler shape as `dispatch`/`warm`. It belongs in
        // this group specifically because a settling can run 55–94s, which exceeds the public
        // function's 60s ceiling — the very reason `api/internal` exists.
        .route(
            "/api/region/dispatch",
            get(handlers::region::dispatch).post(handlers::region::dispatch),
        )
}

/// The webhook intake transport — unauthenticated at the middleware and **self-gated on the
/// broker attestation**, the same posture as [`embed_internal_routes`] and for the same reason:
/// the caller is not a temper principal and holds no temper token. Vercel Connect forwards a
/// remote system's event carrying an RS256 attestation, and the handler verifies it itself (signature, issuer, audience, the anti-decoy `client_id`, and the signed
/// `trigger` claim) before anything reaches the database.
///
/// A router of its own rather than a route on [`embed_internal_routes`] because the controls are
/// not the same control. That group self-checks a shared secret temper issues; this one verifies
/// a third party's signature against a remote JWKS. Folding them together would put a route whose
/// gate is a bearer comparison in the same reviewed set as one whose gate is a JWKS verification,
/// and the tripwire's whole job is that each entry names the control it actually carries.
///
/// **`create_app` only.** Connect forwards to the public host; `create_internal_app` exists solely
/// to give Vercel crons a longer `maxDuration` and serves no third-party surface. Same reasoning
/// as [`slack_link_public_routes`].
///
/// The body limit is set here and — until `query_routes` `[2026-08-28]` — nowhere else. Nothing in
/// temper-api set `DefaultBodyLimit` at the time, so axum's 2 MB default applied — under GitHub's 25 MB ceiling, above which GitHub *silently drops*
/// the delivery. A payload between the two was refused by temper while GitHub believed it had
/// delivered. Scoped to this route rather than raised globally: no other endpoint has a reason to
/// accept a 25 MB body, and a global raise would hand every one of them the same exposure. Note it
/// bounds the body axum's extractor sees, which is *after* the app-wide
/// `RequestDecompressionLayer` — so it bounds decompressed bytes, which is the direction that
/// matters.
///
/// Excluded from the OpenAPI contract entirely: the caller is Connect, which reads no spec of ours.
fn webhook_intake_routes() -> Router<AppState> {
    use axum::extract::DefaultBodyLimit;
    use axum::routing::post;

    Router::new()
        .route(
            "/api/intake/webhook",
            post(handlers::webhook_intake::receive),
        )
        .layer(DefaultBodyLimit::max(GITHUB_MAX_WEBHOOK_BYTES))
}

/// GitHub's documented webhook payload ceiling. Above this GitHub does not deliver at all, so
/// accepting more would buy nothing while widening what one request can make temper hold in
/// memory.
const GITHUB_MAX_WEBHOOK_BYTES: usize = 25 * 1024 * 1024;

/// `/api/query` alone, carrying the one bound on a composition that the schema cannot express.
///
/// **Merged into [`gated_routes`] rather than mounted there, purely so the layer is scoped.** A
/// `DefaultBodyLimit` applies to every route in the router it is attached to, and no other gated
/// route has any reason to accept a body this size — the same argument that keeps
/// [`webhook_intake_routes`] separate, and the reason this is a merge rather than one more
/// `.routes(...)` line. It stays inside `gated_routes`' auth and system-access layers, which are
/// applied to the merged whole in [`create_app`]; nothing about the mounting changes who may knock.
fn query_routes() -> OpenApiRouter<AppState> {
    use axum::extract::DefaultBodyLimit;

    OpenApiRouter::new()
        .routes(routes!(handlers::query::query))
        .layer(DefaultBodyLimit::max(QUERY_MAX_BODY_BYTES))
}

/// `/api/blobs/uploads/{id}/segments` alone — the one blob door whose request body is raw
/// segment bytes. The staging ceiling (the cumulative bound across appends) is
/// `BlobConfig::max_bytes`, enforced with its own vocabulary in the service; this layer
/// exists so the per-request body is bounded by the platform's own ceiling rather than
/// axum's inherited 2 MB default, which would refuse legal segments in a door whose whole
/// point is bodies past the single-request threshold. Same scoping argument as
/// [`query_routes`]: a `DefaultBodyLimit` applies to every route in the router it is
/// attached to, and no other blob route accepts a body at all. It stays inside
/// `gated_routes`' auth layers, applied to the merged whole in [`create_app`].
const BLOB_SEGMENT_MAX_BODY_BYTES: usize = 4_500_000;

fn blob_segment_routes() -> OpenApiRouter<AppState> {
    use axum::extract::DefaultBodyLimit;

    OpenApiRouter::new()
        .routes(routes!(handlers::blobs::append_segment))
        .layer(DefaultBodyLimit::max(BLOB_SEGMENT_MAX_BODY_BYTES))
}

/// The largest composition `/api/query` will read.
///
/// # Declared because the inherited number is wrong, not merely because inheriting is untidy
///
/// Nothing in temper-api set `DefaultBodyLimit`, so axum's 2 MB default was this door's operative
/// bound by accident. `MAX_PER_CANDIDATE_PROBES`' own doc already reasons *against* that number as
/// a bound — *"the list fits in a fraction of axum's default 2 MB body limit"* — which is the tell
/// that it was doing work nobody had chosen.
///
/// **And it is wrong in the direction that refuses legal plans.** A caller may send a precomputed
/// 768-float embedding beside each question (`Intention::embedding`, which the CLI always does),
/// and that is ~10 KB per stage on the wire. At `MAX_STAGES` stages, with a question at
/// `MAX_INTENTION_QUERY_BYTES` and bounds at `MAX_ID_SET_IDS`, a composition the contract calls
/// legal serializes to **2,194,320 bytes** `[measured — 2026-08-28, by the test named below]` —
/// 97 KB past the inherited 2,097,152. So the door would have answered a plan its own contract
/// admits with a bare 413: no refusal list, no vocabulary, in the door whose whole promise is that
/// every refusal arrives at once and in the caller's own terms.
/// `the_largest_legal_composition_fits_inside_the_declared_body_limit` holds that, and fails
/// against the inherited number rather than merely describing it.
///
/// # Why 4 MB and not the sum
///
/// **Every COUNT the contract admits is now bounded, and what remains unbounded is LENGTH.**
/// `[narrowed — 2026-08-28, after review]` This paragraph first named only the length half, which
/// understated why the backstop was needed: `ReturnSpec::with`, `EdgeFilter::edge_kinds`,
/// `EdgeFilter::labels`, `ResourceFilter::doc_type` and `ResourceFilter::tags` were `Vec`s no pass
/// capped, and `validate_returns` checked section MEMBERSHIP only — so `with: [open_meta; 10_000]`
/// per return validated `Ok` and serialized to **9.6 MB**, and ten thousand one-character labels
/// per stage to **4.7 MB**. Both are refused now: `MAX_FILTER_VALUES` bounds the three open lists,
/// and `DuplicateSetMember` bounds the two closed vocabularies at their own size.
///
/// So the coherence property below holds over every field whose COUNT the contract fixes, and
/// `the_largest_legal_composition_fits_inside_the_declared_body_limit` measures that maximum at
/// **2,455,972 bytes** — 1.71x under this number.
///
/// What it does not bound is SIZE, and there are two kinds `[both named — 2026-08-28, after review]`:
/// the LENGTH of a string inside a counted list (a facet key, a label, a `title_contains`), and the
/// serialized size of a single `Contains` VALUE — `probe_count` charges one probe per value however
/// large, so one value holding a million-element JSON array is 6.9 MB and validates `Ok`.
///
/// Through the counted lists, reaching 4 MB now takes ~400 bytes per label across every stage
/// rather than one byte, which is the difference between a caller and an adversary. Through a
/// `Contains` value it takes a single field. That second one is the thing to bound next, and it is
/// why this limit is a backstop and not a sum.
pub const QUERY_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

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

    // The rate-limit layer is added BEFORE the signature gate so it sits INSIDE it:
    // layers run outermost-first, so the signature check runs first on every request,
    // and an unsigned caller cannot spend the signed caller's budget — garbage gets the
    // 401 and the counter never hears of it. With the seam unconfigured (`rate_limit:
    // None`) the middleware passes straight through, so this wiring is inert by default.
    let internal = internal_routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            temper_services::rate_limit::require_route_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_internal_signature,
        ));

    let slack_link_internal =
        slack_link_internal_routes().layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_slack_link_signature,
        ));

    let slack_mint_internal =
        slack_mint_internal_routes().layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_slack_mint_signature,
        ));

    let embed_internal = embed_internal_routes();

    let mut app = Router::new()
        .merge(public)
        .merge(auth_only)
        .merge(gated)
        .merge(internal)
        .merge(slack_link_internal)
        .merge(slack_mint_internal)
        .merge(slack_link_public_routes())
        .merge(embed_internal)
        .merge(webhook_intake_routes());

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

/// The internal/system-only app — the two non-user-auth surfaces (`internal_routes`,
/// self-gated by the internal-signature middleware, and `embed_internal_routes`,
/// self-gated by `EMBED_DISPATCH_SECRET`) with the same transport layers as
/// [`create_app`], but none of the public/auth/gated API and no Swagger.
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
    // Same layering as `create_app`'s `internal`: rate limit innermost, signature
    // outermost, per the merge-site discipline — the internal function must carry the
    // seam identically to the public one, or the second serving path would be the
    // unlimited copy of the first.
    let internal = internal_routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            temper_services::rate_limit::require_route_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_internal_signature,
        ));
    let slack_link_internal =
        slack_link_internal_routes().layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_slack_link_signature,
        ));
    let slack_mint_internal =
        slack_mint_internal_routes().layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_slack_mint_signature,
        ));
    let embed_internal = embed_internal_routes();

    let app = Router::new()
        .merge(internal)
        .merge(slack_link_internal)
        .merge(slack_mint_internal)
        .merge(embed_internal);

    apply_transport_layers(app, state)
}

/// Apply the shared transport-layer stack (fallback, request decompression, HTTP
/// tracing, CORS) and bind `state`. Shared by [`create_app`] and
/// [`create_internal_app`] so both surfaces observe and trace requests identically.
fn apply_transport_layers(app: Router<AppState>, state: AppState) -> Router {
    let cors = temper_services::cors::cors_layer(&state.config);

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
    // Same reason, for the component half: an open enum reached only through a route's
    // request/response body is not in `components` until the merges above collect it, so a
    // modifier registered on `ApiDoc` would run before the schema it repairs exists.
    crate::openapi::OpenStringEnumAddon.modify(&mut spec);
    crate::openapi::ApidogFolderAddon.modify(&mut spec);
    spec
}

#[cfg(test)]
mod tests {
    use super::QUERY_MAX_BODY_BYTES;
    use std::collections::BTreeMap;
    use temper_core::types::graph::EdgeKind;
    use temper_core::types::query::act::ActName;
    use temper_core::types::query::composition::{
        Composition, Intention, OutcomeDeclaration, ReturnSpec, StageNode,
        MAX_INTENTION_QUERY_BYTES, MAX_STAGES,
    };
    use temper_core::types::query::envelope::ActInvocation;
    use temper_core::types::query::filter::{
        EdgeFilter, FacetPredicate, PropertyOp, PropertyPredicate, ResourceFilter,
        MAX_FILTER_VALUES,
    };
    use temper_core::types::query::id_set::{IdKind, IdSet, MAX_ID_SET_IDS};
    use temper_core::types::query::scalars::BoundTerm;
    use temper_core::types::query::stage::{StageInput, StageName, StageRelation};
    use temper_core::types::query::validate::validate;
    use temper_core::types::resource_view::ResourceSection;

    /// **The coherence condition that makes the caps one decision rather than several ifs.**
    ///
    /// Every bound on what a request may declare is published on the field it bounds and refused in
    /// the shape pass, with a typed reason and every sibling refusal beside it. The body limit is
    /// the one such bound the schema cannot carry, so it is declared here — and it is only
    /// coherent with the others if a caller never meets it while inside them. A composition at
    /// every published cap that does not FIT is a plan the contract calls legal and the transport
    /// answers with a bare 413: no refusal list, no vocabulary, in the door whose whole promise is
    /// that a plan is repaired in one round trip.
    ///
    /// **The plan is VALIDATED before it is measured, and that assertion is not ceremony**
    /// `[added — 2026-08-28, found in review]`. The first version of this fixture used
    /// `find-about-anywhere` and carried a seed and a bound — an act that declares
    /// `accepts_bounds: vec![]` and `accepts_seeds: vec![]` (`registry.rs:202-203`, *"a bound would
    /// make this find-about-within"*). So it measured a plan carrying **128 refusals** while its
    /// own doc called it *"a plan the contract calls legal"*, and nothing asked. The size claim
    /// happened to survive — `follow-from` is the one act declaring both, and swapping it moved the
    /// total by 512 bytes — but a size measured over an illegal plan proves nothing about what the
    /// door must accept, and the next edit to this fixture would have had no guard at all.
    ///
    /// The dominant term is the **caller id sets**, at roughly twice the embeddings
    /// `[measured — 2026-08-28]`: 1,286,912 bytes against 639,872, with the questions third at
    /// 262,144. Named because both were misattributed here first — strip the id sets and the
    /// fixture is 907,408 bytes, comfortably inside the inherited limit, so they are what carries
    /// it past.
    ///
    /// **What it does NOT prove**, stated because a green here reads like completeness. Two things:
    ///
    /// - **It is a floor, not the maximum.** Every COUNT the contract admits is bounded as of
    ///   2026-08-28, so what escapes is SIZE: the length of a string inside a counted list (a facet
    ///   key, a label, a `title_contains`), and the serialized size of a `Contains` VALUE, which
    ///   `probe_count` counts as one probe however large — a single value holding a million-element
    ///   JSON array is 6.9 MB and validates `Ok` `[measured — 2026-08-28]`. Reaching the limit
    ///   through the counted lists now takes ~400 bytes per label across every stage instead of
    ///   one; through a `Contains` value it takes one.
    /// - **The headline number is the WALK shape, which admits no `ResourceFilter`** — so
    ///   `doc_type` and `tags` at their cap appear only in the selection shape, which is half the
    ///   size. No single act admits every bounded field, which is why both are measured; but the
    ///   maximum reported is not maximal in those two fields.
    ///
    #[test]
    fn the_largest_legal_composition_fits_inside_the_declared_body_limit() {
        // **Two shapes, both measured, because no single act admits every bounded field and the
        // larger one is not obvious.** `follow-from` takes a seed, a bound and an `EdgeFilter`;
        // `find-resources-with` is the only act whose `ResourceFilter` is not refused
        // (`capability.rs`'s narrowings block), and it accepts no bounds and no page terms at all.
        // A first version mixed them and measured 1,756,196 — LESS than either pure shape, because
        // half its stages carried no id sets. Measuring both and taking the larger is what stops
        // this test from quietly reporting a maximum that is not one.
        let walk = plan_of(MAX_STAGES, ActName::FollowFrom);
        let select = plan_of(MAX_STAGES, ActName::FindResourcesWith);

        for (what, c) in [("walk", &walk), ("selection", &select)] {
            // Legal FIRST. A byte count over a plan the validator refuses is a measurement of
            // nothing — and the first version of this test measured one carrying 128 refusals
            // while its own doc called it legal `[found in review — 2026-08-28]`.
            assert!(
                validate(c).is_ok(),
                "the {what} fixture must be a composition this server would RUN, or its size says \
                 nothing about what the door has to accept: {:?}",
                validate(c).err()
            );
        }

        let sizes: Vec<usize> = [&walk, &select]
            .iter()
            .map(|c| {
                serde_json::to_vec(c)
                    .expect("a composition serializes")
                    .len()
            })
            .collect();
        let bytes = *sizes.iter().max().expect("two shapes");
        assert!(
            bytes < QUERY_MAX_BODY_BYTES,
            "the largest composition at every published cap serializes to {bytes} bytes (walk \
             {}, selection {}), which the declared body limit of {QUERY_MAX_BODY_BYTES} would \
             refuse with a bare 413 — raise the limit, or lower the field caps, but do not let the \
             contract admit a plan the door cannot read",
            sizes[0],
            sizes[1]
        );
    }

    /// `n` stages of one act, each maximal over every field that act admits and every cap the
    /// contract publishes.
    ///
    /// **A selection-shaped plan still ends in one walk stage**, because a selection orders nothing
    /// and is refused in `returns` (`StageNotReturnable`) while a composition that returns nothing
    /// is refused outright (`NoReturns`). So the pure shape is not legal at any size, and the
    /// largest selection-shaped plan is `n - 1` selections plus the walk that answers.
    fn plan_of(n: usize, act: ActName) -> Composition {
        let all_walk = act == ActName::FollowFrom;
        let stages: Vec<StageNode> = (0..n)
            .map(|i| {
                let walk = all_walk || i == n - 1;
                StageNode::Act(ActInvocation {
                    // Stage names at their own ceiling — 63 (`stage.rs:43`).
                    name: StageName::parse(&format!(
                        "s{i}{}",
                        "n".repeat(60 - i.to_string().len())
                    ))
                    .expect("legal stage name"),
                    act: if walk {
                        ActName::FollowFrom
                    } else {
                        act.clone()
                    },
                    intention: Some(Intention {
                        query: "x".repeat(MAX_INTENTION_QUERY_BYTES),
                        // A real normalized BGE component, so the serialized width is the one a
                        // caller actually sends rather than the two bytes `0.0` would cost.
                        //
                        // Every stage carries one, which is also what keeps this inside
                        // `MAX_COMPOSITION_INTENTION_BYTES`: that bound counts only what the SERVER
                        // must embed, and a caller who precomputed has already paid it. A fixture
                        // without embeddings is a DIFFERENT and SMALLER maximum, because the
                        // aggregate budget then caps its question text at 64 KB.
                        embedding: Some(vec![-0.041_899_003; 768]),
                    }),
                    inputs: if walk {
                        vec![
                            StageInput::Caller {
                                relation: StageRelation::Seed,
                                ids: full_id_set(),
                            },
                            StageInput::Caller {
                                relation: StageRelation::Bound,
                                ids: full_id_set(),
                            },
                        ]
                    } else {
                        // A selection accepts no bounds of any kind and no page terms — it declares
                        // a set. Both are `capability`'s refusals, and hitting them is how this
                        // fixture learned the shape rather than assuming it.
                        vec![]
                    },
                    terms: if walk {
                        BTreeMap::from([(BoundTerm::Limit, 50), (BoundTerm::Offset, 50)])
                    } else {
                        BTreeMap::new()
                    },
                    resource_filter: (!walk).then(full_resource_filter),
                    edge_filter: walk.then(full_edge_filter),
                    properties: vec![],
                })
            })
            .collect();

        Composition {
            outcome: OutcomeDeclaration {
                // A selection orders nothing and is refused in `returns` (`StageNotReturnable`), so
                // that shape returns its first stage only — which is what a caller would do.
                // Every walk stage, which for the walk shape is all of them and for the selection
                // shape is the one that answers.
                returns: stages
                    .iter()
                    .filter(|n| matches!(n, StageNode::Act(i) if i.act == ActName::FollowFrom))
                    .map(|node| ReturnSpec {
                        stage: node.name().clone(),
                        with: vec![ResourceSection::OpenMeta],
                    })
                    .collect(),
            },
            stages,
        }
    }

    /// Every narrowing list at [`MAX_FILTER_VALUES`], and both per-candidate containers at the caps
    /// `capability.rs` enforces — 32 predicates summing to 256 probes.
    fn full_resource_filter() -> ResourceFilter {
        ResourceFilter {
            doc_type: vec!["d".to_string(); MAX_FILTER_VALUES],
            tags: vec!["t".to_string(); MAX_FILTER_VALUES],
            facets: (0..16)
                .map(|i| FacetPredicate {
                    key: format!("k{i}"),
                    value: "v".to_string(),
                })
                .collect(),
            // 16 facets + 16 predicates = 32, the predicate cap; 16 facets + 240 probes = 256,
            // the probe cap. Facets count against BOTH, which is what the container's own doc
            // means by summing what walks the same candidate set.
            properties: capped_properties(16, 15),
            stage: Some("s".to_string()),
            status: Some("a".to_string()),
            owner: Some("o".to_string()),
            title_contains: Some("t".to_string()),
        }
    }

    fn full_edge_filter() -> EdgeFilter {
        EdgeFilter {
            // A closed vocabulary carried as a list, and repeats are refused — so its ceiling IS
            // the vocabulary, and naming every member is what makes this maximal. `[widened from
            // one — 2026-08-28, found in review]`
            edge_kinds: vec![
                EdgeKind::Express,
                EdgeKind::Contains,
                EdgeKind::LeadsTo,
                EdgeKind::Near,
            ],
            labels: vec!["l".to_string(); MAX_FILTER_VALUES],
            // No facets on an edge container, so all 32 predicates and all 256 probes are the
            // property list's.
            properties: capped_properties(32, 8),
        }
    }

    /// `preds` predicates each carrying `vals` values. The two caps a container must satisfy are
    /// `MAX_PER_CANDIDATE_PREDICATES` (32, summed with `facets` where the container has them) and
    /// `MAX_PER_CANDIDATE_PROBES` (256, likewise) — so the split differs between the two containers
    /// and is passed rather than assumed.
    fn capped_properties(preds: usize, vals: usize) -> Vec<PropertyPredicate> {
        (0..preds)
            .map(|i| PropertyPredicate {
                key: format!("p{i}"),
                op: PropertyOp::Contains {
                    values: (0..vals)
                        .map(|v| serde_json::json!(format!("v{v}")))
                        .collect(),
                },
            })
            .collect()
    }

    fn full_id_set() -> IdSet {
        IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: (0..MAX_ID_SET_IDS).map(|_| uuid::Uuid::now_v7()).collect(),
        }
    }
}
