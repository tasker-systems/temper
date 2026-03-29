use std::env;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub database_url: String,
    pub jwks_url: String,
    pub auth_issuer: String,
    pub auth_audience: Option<String>,
    pub auth_provider_name: String,
    pub cors_origins: Vec<String>,
    pub port: u16,
    pub enable_swagger: bool,
    pub r2_account_id: Option<String>,
    pub r2_access_key_id: Option<String>,
    pub r2_secret_access_key: Option<String>,
    pub r2_bucket_name: Option<String>,
    pub r2_public_base_url: Option<String>,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, env::VarError> {
        let cors_origins: Vec<String> = env::var("CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let auth_audience = env::var("AUTH_AUDIENCE").ok().filter(|s| !s.is_empty());

        let auth_provider_name =
            env::var("AUTH_PROVIDER_NAME").unwrap_or_else(|_| "neon_auth".to_string());

        if auth_audience.is_none() {
            tracing::warn!(
                "AUTH_AUDIENCE is not set — JWT audience validation is disabled. \
                 This is acceptable for development but MUST be configured in production."
            );
        }

        if cors_origins.is_empty() {
            tracing::info!(
                "CORS_ORIGINS is not set — cross-origin requests will be denied. \
                 Set CORS_ORIGINS=* for permissive mode in development."
            );
        }

        let enable_swagger = env::var("ENABLE_SWAGGER")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        if enable_swagger {
            tracing::info!("Swagger UI enabled at /api-docs/ui");
        }

        let r2_account_id = env::var("R2_ACCOUNT_ID").ok().filter(|s| !s.is_empty());
        let r2_access_key_id = env::var("R2_ACCESS_KEY_ID").ok().filter(|s| !s.is_empty());
        let r2_secret_access_key = env::var("R2_SECRET_ACCESS_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let r2_bucket_name = env::var("R2_BUCKET_NAME").ok().filter(|s| !s.is_empty());
        let r2_public_base_url = env::var("R2_PUBLIC_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty());

        if r2_account_id.is_none() || r2_bucket_name.is_none() {
            tracing::info!("R2 not configured — file upload endpoints will be unavailable");
        }

        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            jwks_url: env::var("JWKS_URL")?,
            auth_issuer: env::var("AUTH_ISSUER")?,
            auth_audience,
            auth_provider_name,
            cors_origins,
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            enable_swagger,
            r2_account_id,
            r2_access_key_id,
            r2_secret_access_key,
            r2_bucket_name,
            r2_public_base_url,
        })
    }
}
