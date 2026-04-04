//! Vercel serverless function entry point for the temper MCP server.
//!
//! Bridges the axum Router from temper-mcp to Vercel's serverless interface
//! via VercelLayer — identical pattern to api/axum.rs.

use sqlx::postgres::PgPoolOptions;
use tower::ServiceBuilder;
use tracing_subscriber::EnvFilter;
use vercel_runtime::axum::VercelLayer;

use temper_api::config::ApiConfig;
use temper_api::state::{AppState, JwksKeyStore};
use temper_mcp::McpConfig;

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let api_config = ApiConfig::from_env().expect("Failed to load ApiConfig from environment");
    let mcp_config = McpConfig::from_env().expect("Failed to load McpConfig from environment");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&api_config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(api_config.jwks_url.clone());
    let api_state = AppState::new(pool, jwks_store, api_config);

    let app = temper_mcp::build_router(api_state, mcp_config);

    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(app);

    tracing::info!("temper-mcp: Vercel function initialized");

    vercel_runtime::run(service).await
}
