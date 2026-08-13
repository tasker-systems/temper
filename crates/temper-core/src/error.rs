use thiserror::Error;

/// The wire `error.code` a [`TemperError::ForbiddenDetail`] travels under — a `403` whose message
/// is load-bearing rather than the constant `"Forbidden"`.
///
/// It lives here, in the crate both sides already depend on, rather than as a literal spelled once
/// in `temper-services`' `IntoResponse` and again in `temper-client`'s status mapper. The sibling
/// `"CONTENT_INTEGRITY"` is spelled that second way and is the reason not to: a code the producer
/// and the consumer each name independently is a wire contract nothing checks.
pub const FORBIDDEN_DETAIL_CODE: &str = "FORBIDDEN_DETAIL";

/// The wire `error.code` a refused composition travels under — a `400` whose `details` carry
/// [`crate::types::error_details::PlanRefusalDetails`], every static refusal at once.
///
/// **A code of its own rather than `BAD_REQUEST`.** The client branches on the code to decide
/// whether a body carries refusals; reusing the generic code would force it to sniff the shape of
/// `details` instead, which is the message-text heuristic in another costume.
///
/// Spelled here for the same reason as [`FORBIDDEN_DETAIL_CODE`] — the producer
/// (`temper-services`' `IntoResponse`) and the consumer (`temper-client`'s status mapper) name one
/// constant rather than two literals nothing checks.
///
/// `[decided — 2026-08-13, Pete]` The spec (§B) authorized "its own code" and named no string.
pub const PLAN_REFUSED_CODE: &str = "PLAN_REFUSED";

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

    /// A `403` that **names the capability it refused** — admissible only where the caller already
    /// holds READ standing on the same subject, so the detail discloses nothing a successful read
    /// would not have told them. Same status and same class as [`Self::Forbidden`]; it differs only
    /// in carrying a message, and travels the wire under [`FORBIDDEN_DETAIL_CODE`].
    ///
    /// **[`Self::Forbidden`] stays the default, and stays argument-free.** That is what keeps *"a
    /// refusal cannot name the subject it refused"* a property of the type rather than of everyone
    /// remembering — the same reasoning `ScopedAuthority::denial` records for its static signature.
    /// Producing this variant on a path that has NOT probed the subject's own read predicate turns
    /// the refusal into an existence oracle, which is the whole thing the terse arm exists to
    /// prevent.
    ///
    /// The precedent is `ContextAdminAuthority`, which splits `ReadOnly → 403` from
    /// `Invisible → 404` on exactly this reasoning: *"the 403 is not an existence oracle — it
    /// reaches only principals who already read the context."* This variant is what lets a gate
    /// that is not a `ScopedAuthority` say the same thing.
    #[error("{0}")]
    ForbiddenDetail(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("system access required")]
    SystemAccessRequired(Box<CliAccessDetails>),
}

pub type Result<T> = std::result::Result<T, TemperError>;
