use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use temper_api::routes::create_app;
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() {
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
}
