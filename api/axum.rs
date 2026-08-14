//! Vercel serverless function entry point for temper-api.
//!
//! This binary bridges the axum Router from temper-api to Vercel's
//! serverless function interface via VercelLayer.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tower::ServiceBuilder;
use vercel_runtime::axum::VercelLayer;

use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    // Name this executable in exported spans before the exporter is built. temper-cloud runs three
    // Rust executables and eight Node lambdas in one Vercel project, so a project-scoped
    // `OTEL_SERVICE_NAME` cannot name them all distinctly — each Rust binary claims its own name here,
    // leaving that env var free for the project's Node half (work item 2b).
    temper_telemetry::set_service_name("temper-api");
    temper_telemetry::init_server_logging();

    // `unwrap_or_else(panic!)` rather than `.expect()`: expect prints Debug, and these errors carry
    // their remedy in Display. An instance that cannot state which audience it validates must not
    // serve traffic.
    let config = ApiConfig::from_env().unwrap_or_else(|e| panic!("refusing to start: {e}"));

    // Bound connection acquisition so a cold Neon compute-resume fails fast
    // rather than hanging the whole serverless invocation window until Vercel
    // kills it. A normal resume is sub-second to a few seconds; 8s leaves
    // headroom under the function timeout. The client retries the resulting
    // transient error (temper-client `should_retry`), so the next invocation
    // hits a warm DB.
    //
    // **`acquire_timeout` IS NOT AN EXECUTION BOUND, and reading it as one inverts what it does.**
    // It bounds how long *this* caller waits to be handed a connection. It says nothing about how
    // long a statement may run once it has one — a query that acquires instantly and then runs for
    // ten minutes never touches this setting. So its effect under load is the opposite of
    // protective: while one expensive statement occupies a connection, `acquire_timeout` is what
    // makes the *next* caller fail, faster, having done nothing wrong.
    //
    // Nothing in this deployment bounds execution. `[measured — 2026-08-14, prod, read-only]`
    // `statement_timeout = 0` and `lock_timeout = 0`, both `source = default`; the only ambient
    // bound is `idle_in_transaction_session_timeout = 300000`, which comes from Neon's
    // configuration file rather than from us and governs a different axis (an idle open
    // transaction, not a running statement).
    //
    // Picking that execution bound is deliberately deferred until it can be measured rather than
    // guessed — see task `01a000ee-9fec-7283-baa5-75cd1580f023` and migration `20260814000020`,
    // which installs the `pg_stat_statements` the measurement needs. Note for whoever takes it:
    // the obvious `.after_connect(… SET statement_timeout …)` hook is the wrong instrument here,
    // because runtime traffic goes through Neon's pooled endpoint, which is "PgBouncer in
    // transaction mode, which does not keep a session pinned to a client"
    // (`scripts/vercel-build.sh:65-66`) — session-level state set that way is not reliably the
    // state a later transaction sees.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(8))
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(config.auth.jwks_url.clone());
    let state = AppState::new(pool, jwks_store, config);
    let app = temper_api::create_app(state);

    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(app);

    tracing::info!("temper-cloud: Vercel function initialized");

    vercel_runtime::run(service).await
}
