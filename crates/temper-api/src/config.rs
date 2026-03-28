use std::env;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub database_url: String,
    pub jwks_url: String,
    pub auth_issuer: String,
    pub auth_audience: Option<String>,
    pub cors_origins: Vec<String>,
    pub port: u16,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, env::VarError> {
        let cors_origins = env::var("CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            jwks_url: env::var("JWKS_URL")?,
            auth_issuer: env::var("AUTH_ISSUER")?,
            auth_audience: env::var("AUTH_AUDIENCE").ok().filter(|s| !s.is_empty()),
            cors_origins,
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
        })
    }
}
