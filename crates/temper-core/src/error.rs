use thiserror::Error;

/// Details from a system access gate rejection (CLI error rendering).
///
/// Distinct from `types::access_gate::SystemAccessDetails` which carries
/// serde derives for API serialization. This version uses plain strings
/// because it arrives via the client error chain (already deserialized).
#[derive(Debug)]
pub struct CliAccessDetails {
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// The typed refusal the server sent on the 403. `Option` only because the client error chain
    /// reconstructs it defensively; every current server populates it.
    pub refusal: Option<temper_principal::Refusal>,
    pub request_url: Option<String>,
    pub cli_command: Option<String>,
}

#[derive(Error, Debug)]
pub enum TemperError {
    #[error("Vault not found — run `temper init` or set TEMPER_VAULT")]
    VaultNotFound,

    #[error("Config error: {0}")]
    Config(String),

    #[error("Vault error: {0}")]
    Vault(String),

    #[error("Project error: {0}")]
    Project(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Index error: {0}")]
    Index(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),
    /// A finalize raw-bytes integrity check failed — the stored bytes do not match the caller's
    /// declared hash (W2 PR 5). Distinct from `Conflict` because it is **not** resumable: the caller
    /// (e.g. the CLI's segmented upload) must discard the poisoned resource and re-upload, not retry.
    #[error("{0}")]
    ContentIntegrity(String),

    #[error("Forbidden")]
    Forbidden,

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("system access required")]
    SystemAccessRequired(Box<CliAccessDetails>),
}

pub type Result<T> = std::result::Result<T, TemperError>;
