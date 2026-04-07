use thiserror::Error;

/// Details from a system access gate rejection.
#[derive(Debug)]
pub struct SystemAccessDetails {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub access_mode: String,
    pub join_request_status: Option<String>,
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

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("system access required")]
    SystemAccessRequired(Box<SystemAccessDetails>),
}

pub type Result<T> = std::result::Result<T, TemperError>;
