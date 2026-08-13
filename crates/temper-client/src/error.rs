use std::time::Duration;

use temper_core::error::CliAccessDetails;
use temper_core::types::query::validate::PlanRefusal;

/// Render every refusal, one per line, for [`ClientError::PlanRefused`]'s `Display`.
///
/// **The list is rendered here rather than only by the command**, because a caller that does not
/// branch on this variant still reaches `Display` — `temper-cli`'s `client_err_to_temper` maps any
/// unmatched `ClientError` to `TemperError::Api(e.to_string())`. Putting the refusals only in the
/// command's renderer would mean every other path silently reports "the plan was refused" and
/// drops the reasons, which is the exact loss this variant exists to prevent.
fn render_refusals(refusals: &[PlanRefusal]) -> String {
    refusals
        .iter()
        .map(|r| match &r.stage {
            Some(stage) => format!("\n  {}: {}", stage.as_str(), r.detail),
            None => format!("\n  {}", r.detail),
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("not authenticated — run `temper auth login`")]
    NotAuthenticated,

    #[error("token expired")]
    TokenExpired,

    #[error("forbidden")]
    Forbidden,

    /// A `403` whose message is load-bearing — the server named the capability it refused, which it
    /// does only for a caller who already reads the subject (see
    /// [`temper_core::error::TemperError::ForbiddenDetail`]).
    ///
    /// Discriminated by the wire `code` being [`temper_core::error::FORBIDDEN_DETAIL_CODE`], exactly
    /// as the `422` arm discriminates `CONTENT_INTEGRITY` — never by sniffing whether the message
    /// happens to differ from the constant `"Forbidden"`.
    ///
    /// Renders bare, like [`Self::NotFound`]: the server's sentence is the whole message, so
    /// `client_err_to_temper` carries it through to the CLI without a label stacked on a label.
    #[error("{message}")]
    ForbiddenDetail { message: String },

    #[error("system access required")]
    SystemAccessRequired(Box<CliAccessDetails>),

    /// Carries the server's own message verbatim and renders it bare.
    ///
    /// This was `{ resource: String }`, parsed from an `error.resource` field the server has
    /// never emitted — `openapi.json` publishes `code`, `message`, `details` and nothing else —
    /// so every real 404 fell through to the `"unknown"` default and rendered `unknown not
    /// found`. It now reads `error.message`, exactly as the 409, 422 and 5xx arms beside it
    /// always have.
    #[error("{message}")]
    NotFound { message: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    /// A `400` from `POST /api/query`: the composition will not run, and **every** static reason
    /// came back at once.
    ///
    /// A **caller** error, deliberately not routed through [`Self::Server`]. Before this variant a
    /// 400 fell to the status catch-all and arrived as `Server { status: 400 }` — the refusal list
    /// discarded and a caller fault reported to the user as a server fault.
    ///
    /// Keeping it a `Vec` rather than a joined string is the whole point. `validate` returns every
    /// refusal rather than the first *"because a caller repairing a plan should see all of it in
    /// one round trip"*; that property is real for raw HTTP and absent for the CLI unless the
    /// client carries the list through structured.
    ///
    /// Discriminated by the wire `code` being [`temper_core::error::PLAN_REFUSED_CODE`], exactly as
    /// the `403` and `422` arms discriminate theirs — never by sniffing for a `details` object,
    /// which would reclassify the moment another error learned to carry one.
    #[error("the plan was refused:{}", render_refusals(.refusals))]
    PlanRefused { refusals: Vec<PlanRefusal> },

    /// A finalize raw-bytes integrity check failed (HTTP 422, `CONTENT_INTEGRITY`) — the stored bytes
    /// do not match the caller's declared hash (W2 PR 5). Distinct from `Conflict` because it is **not**
    /// resumable: the caller must discard the poisoned resource and re-upload, not retry.
    #[error("content integrity check failed: {message}")]
    ContentIntegrity { message: String },

    #[error("rate limited — retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },

    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },

    /// A required cloud-configuration field (API URL, OAuth callback URL) is
    /// empty. Surfaced before any network attempt so the user gets an
    /// actionable "run `temper init`" message instead of a cryptic reqwest
    /// "builder error" (empty base URL) or an Auth0 "Oops" page (empty
    /// `redirect_uri`). See the regression from baked-in defaults being
    /// removed in favor of per-instance config.
    #[error("{0}")]
    NotConfigured(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl ClientError {
    /// True if this error indicates the server could not be reached
    /// (DNS failure, connection refused, TCP timeout, TLS handshake, etc.).
    /// False for responses from the server itself (4xx/5xx, auth, conflicts).
    pub fn is_network(&self) -> bool {
        matches!(self, ClientError::Network(_))
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;
