//! Vercel serverless function entry point for temper-api.
//!
//! This binary bridges the axum Router from temper-api to Vercel's
//! serverless function interface via VercelLayer.

use sqlx::postgres::PgPoolOptions;
use tower::ServiceBuilder;
use tracing_subscriber::EnvFilter;
use vercel_runtime::axum::VercelLayer;

use temper_api::config::ApiConfig;
use temper_api::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ApiConfig::from_env().expect("Failed to load config from environment");

    let pool = PgPoolOptions::new()
        .max_connections(5)
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
