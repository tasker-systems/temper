use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;

use temper_api::routes::create_app;
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() {
    // The public-API surface, whether run here (local / self-hosted) or as the Vercel `api/axum.rs`
    // executable — same name, so a self-hosted deployment reports `temper-api` exactly as the cloud
    // one does (work item 2b). Must precede init, which builds the exporter that reads it.
    temper_telemetry::set_service_name("temper-api");
    temper_telemetry::init_server_logging();

    // `unwrap_or_else(panic!)` rather than `.expect()`: expect prints Debug, and these errors carry
    // their remedy in Display. An instance that cannot state which audience it validates must not
    // serve traffic.
    let config = ApiConfig::from_env().unwrap_or_else(|e| panic!("refusing to start: {e}"));

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(config.auth.jwks_url.clone());
    let port = config.port;
    let state = AppState::new(pool, jwks_store, config);
    let app = create_app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    tracing::info!("temper-api listening on {addr}");

    axum::serve(listener, app).await.expect("Server failed");

    // This binary is the one deployable that actually exits. The Vercel functions are frozen rather
    // than shut down, which is why they drain per-response instead; here a normal exit can drain
    // once, properly.
    temper_telemetry::shutdown_telemetry();
}
