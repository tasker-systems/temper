//! Vercel serverless function entry point for the temper MCP server.
//!
//! Bridges the axum Router from temper-mcp to Vercel's serverless interface
//! via VercelLayer — identical pattern to api/axum.rs.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tower::ServiceBuilder;
use vercel_runtime::axum::VercelLayer;

use temper_mcp::McpConfig;
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    // Name this executable in exported spans before the exporter is built — see api/axum.rs. The MCP
    // and API executables share a Vercel project, so distinct code-set names are what keep their
    // spans apart in the Tier-2 waterfall (work item 2b).
    temper_telemetry::set_service_name("temper-mcp");
    temper_telemetry::init_server_logging();

    // `unwrap_or_else(panic!)` rather than `.expect()`: expect prints Debug, and these errors carry
    // their remedy in Display. An instance that cannot state which audience it validates must not
    // serve traffic.
    let api_config = ApiConfig::from_env().unwrap_or_else(|e| panic!("refusing to start: {e}"));
    let mcp_config = McpConfig::from_env().expect("Failed to load McpConfig from environment");

    // Bound connection acquisition — same rationale and same value as `api/axum.rs`, including its
    // note that **`acquire_timeout` is not an execution bound**; read that comment before treating
    // this line as protection against an expensive query.
    //
    // This surface shipped without it while `api/axum.rs` and `api/internal.rs` both carried it
    // `[found — 2026-08-14]`. The asymmetry was unintended rather than a considered difference: MCP
    // runs the same `create_app`-shaped stack against the same 5-connection pool and the same
    // 60s `maxDuration` (`vercel.json`), so a cold Neon compute-resume could hang the whole
    // invocation window here in exactly the way the other two chose not to allow. It is also the
    // agent-facing door, so it is the one most likely to be called by something that will retry.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(8))
        .connect(&api_config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(api_config.auth.jwks_url.clone());
    let api_state = AppState::new(pool, jwks_store, api_config);

    let app = temper_mcp::build_router(api_state, mcp_config);

    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(app);

    tracing::info!("temper-mcp: Vercel function initialized");

    vercel_runtime::run(service).await
}
