//! Internal cron-invoked embed (and Slack intents reaper) endpoints — self-gated by
//! EMBED_DISPATCH_SECRET (bearer), NOT `require_auth`. Called by Vercel crons on a schedule; each
//! handler checks the secret itself (fail-closed when unset), so no auth-middleware layer is
//! applied. Excluded from the OpenAPI contract entirely.
//!
//! - `/api/embed/dispatch` — the async-embed drain (issue #299).
//! - `/api/embed/warm` — cold-start warmup for server-side query embedding (issue #427).
//! - `/api/slack/intents/reap` — hourly retention sweep for expired/consumed link intents (T4).
//! - `/api/as/reap` — daily retention sweep for the three Authorization Server tables
//!   (TMPR-56), and — since 2026-09-05 — for abandoned staged blob uploads, which ride
//!   the same cron by ruling (task 01a0715d).
//! - `/api/sensitivity/sweep` — the sensitivity sweep's five-minute tick (sensitivity-sweep spec D8).
//!
//! NOTE: `embed::dispatch`'s `#[utoipa::path]` declares `get` only, but the route
//! mounts BOTH GET and POST on the same handler. This plain `.route()` (rather than
//! `routes!()`) is precisely why it can keep both methods AND stay out of the spec.
//! `slack_disconnect::reap_intents` carries no `#[utoipa::path]` at all, for the same reason.

use axum::routing::get;
use axum::Router;

use crate::handlers;
use temper_services::state::AppState;

pub(super) fn embed_internal_routes() -> Router<AppState> {
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
        // kb_oauth_refresh_tokens, none of which anything had ever deleted from — plus the
        // abandoned-staged-blob-upload TTL reaper (task 01a0715d), which rides this scheduler
        // entry by ruling rather than minting a second cron. Same self-gated
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
        // The erasure byte-delete fence's tick (task 01a0577c Beat 4): derives pending deletes
        // from the `principal_erased` payloads, reaps expired leases, and drains due deletes
        // through one batched `BlobStore::delete`. Same self-gated posture and GET+POST shape
        // as `dispatch`/`warm`; it belongs in this group because the substrate contract's fence
        // ("retry plus age alerting") is only as real as the cron that drives it — vercel.json
        // runs it every minute, the same cadence the retry ladder's backoff curve assumes.
        .route(
            "/api/erasure/drain",
            get(handlers::erasure::drain).post(handlers::erasure::drain),
        )
        // The sensitivity sweep's tick (sensitivity-sweep spec D8, build order 3a PR D): claims one
        // surface's work order and scans it inside its time budget. Same self-gated posture and
        // GET+POST shape as `dispatch`/`warm`. It belongs in this group for the 300s ceiling, and
        // because `require_dispatch_secret` is already set wherever the other crons run.
        .route(
            "/api/sensitivity/sweep",
            get(handlers::sensitivity_sweep::sweep).post(handlers::sensitivity_sweep::sweep),
        )
}
