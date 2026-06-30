//! Vercel serverless function entry point for temper-api.
//!
//! This binary bridges the axum Router from temper-api to Vercel's
//! serverless function interface via VercelLayer.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tower::ServiceBuilder;
use tracing_subscriber::EnvFilter;
use vercel_runtime::axum::VercelLayer;

use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ApiConfig::from_env().expect("Failed to load config from environment");

    // Bound connection acquisition so a cold Neon compute-resume fails fast
    // rather than hanging the whole serverless invocation window until Vercel
    // kills it. A normal resume is sub-second to a few seconds; 8s leaves
    // headroom under the function timeout. The client retries the resulting
    // transient error (temper-client `should_retry`), so the next invocation
    // hits a warm DB.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(8))
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(config.jwks_url.clone());
    let state = AppState::new(pool, jwks_store, config);
    let app = temper_api::create_app(state);

    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(app);

    tracing::info!("temper-cloud: Vercel function initialized");

    vercel_runtime::run(service).await
}
