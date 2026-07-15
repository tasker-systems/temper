//! Vercel serverless function entry point for temper-api's internal/system surface.
//!
//! Serves ONLY the non-user-auth, infrastructure-invoked routes — the embed crons
//! (`/api/embed/dispatch`, `/api/embed/warm`, self-gated by `EMBED_DISPATCH_SECRET`)
//! and the server-to-server internal routes (`/internal/*`, signature-gated). It is a
//! separate Vercel Function from `api/axum.rs` for one reason: Vercel's `maxDuration`
//! is per-function, and the embed crons run ONNX warmups + drain passes that can
//! exceed the 60s public-API ceiling. Isolating them here lets `vercel.json` give this
//! function a longer timeout without letting a public request hang for that window.
//!
//! Same VercelLayer bridge and startup shape as `api/axum.rs`; only the router differs
//! (`create_internal_app` instead of `create_app`).

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

    // `unwrap_or_else(panic!)` rather than `.expect()`: expect prints Debug, and these errors carry
    // their remedy in Display. An instance that cannot state which audience it validates must not
    // serve traffic.
    let config = ApiConfig::from_env().unwrap_or_else(|e| panic!("refusing to start: {e}"));

    // Bound connection acquisition so a cold Neon compute-resume fails fast rather than hanging the
    // whole serverless invocation window until Vercel kills it. Same rationale as `api/axum.rs`.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(8))
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(config.auth.jwks_url.clone());
    let state = AppState::new(pool, jwks_store, config);
    let app = temper_api::create_internal_app(state);

    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(app);

    tracing::info!("temper-cloud: Vercel internal function initialized");

    vercel_runtime::run(service).await
}
