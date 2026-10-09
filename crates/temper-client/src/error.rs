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

    /// A `401` whose body carried the API's error envelope, preserved verbatim.
    ///
    /// The stock `NotAuthenticated` discards the body — the right shape for the CLI,
    /// whose only remedy is re-login, and the shape that destroyed the information the
    /// MCP relay's tool layer needs to speak the right refusal sentence post-edge
    /// (deactivated vs machine-credential refusal vs expired-in-flight all arrive as
    /// 401s with distinct bodies; see the network-door design §3). The variant is an
    /// EXTEND, not a re-voice: its `Display` is **byte-identical** to
    /// [`Self::NotAuthenticated`], so every caller that does not pattern-match — the
    /// CLI above all — renders exactly what it rendered before. The relay matches on
    /// the variant to read `.message`; nobody else needs to know it exists.
    ///
    /// A 401 without a parseable envelope body (the MCP edge's own plain-text
    /// refusal, a platform 401) stays [`Self::NotAuthenticated`] — the preserved-body
    /// channel exists only where the API actually spoke.
    #[error("not authenticated — run `temper auth login`")]
    UnauthorizedDetails { message: String },

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

    /// 410 Gone — the addressed thing PERSISTS but is gone for this operation: a folded
    /// content block under write addressing (the defined-dangling-state design). Distinct
    /// from [`Self::NotFound`] so the folded state is named, never collapsed into "never
    /// existed"; carries the server's sentence verbatim and renders it bare, like NotFound.
    #[error("{message}")]
    Gone { message: String },

    /// 410 under [`temper_core::error::RESOURCE_ERASED_CODE`] — the addressed resource was
    /// ERASED and the caller holds standing on the husk. Distinct from [`Self::Gone`] (a folded
    /// block, whose row survives and can still be addressed) and from [`Self::NotFound`] (which
    /// may be a soft delete, a move, or an unreadable resource): this is the one answer that
    /// licenses a client to drop its local copy.
    ///
    /// Discriminated by the wire `code`, exactly as the 403 arm discriminates
    /// `FORBIDDEN_DETAIL` — never by the message. **Carries the id, not the message**: the
    /// envelope carries no `details` by design (the server's own test asserts it), so the id
    /// rides only in the fixed sentence `resource <id> was erased`, and every consumer that acts
    /// on the variant needs the id as a typed value — the CLI to lift it to
    /// [`temper_core::error::TemperError::ResourceErased`], whose wire code a JSON caller
    /// branches on. Parsing it once, here, against core's own `Display`, is what keeps that
    /// parse in one place; a sentence that does not parse is a contract disagreement, reported
    /// as such (see `map_status_to_error`), never guessed at. Renders exactly as core's variant.
    #[error("resource {id} was erased")]
    ResourceErased {
        id: temper_core::types::ids::ResourceId,
    },

    #[error("conflict: {message}")]
    Conflict { message: String },

    /// An append or finalize on an ingest that has ended (HTTP 409, code
    /// [`temper_core::error::INGEST_ENDED_CODE`]): the resource's ingest is `cancelled` or
    /// `abandoned`. Distinct from [`Self::Conflict`] because it is **not** resumable: a resuming
    /// caller drops its resume record and starts a new upload. Discriminated by the wire `code`,
    /// as the `422` arm discriminates `CONTENT_INTEGRITY` — never by sniffing the message.
    #[error("{message}")]
    IngestEnded { message: String },

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

    /// The server could not read the body as a composition — malformed JSON, a value of the wrong
    /// type, a name outside a closed vocabulary — so there is no plan and no refusal list. A
    /// **caller** error, like [`Self::PlanRefused`], and the message is the server's own account of
    /// what failed (bounded by the server).
    ///
    /// Discriminated by the wire `code` being [`temper_core::error::UNREADABLE_PLAN_CODE`], at any
    /// status the reader chose. A caller that sends plans it never parsed — the MCP edge relays
    /// them unread — depends on this to report the server's sentence rather than a server fault.
    #[error("the plan could not be read: {message}")]
    UnreadablePlan { message: String },

    /// A finalize raw-bytes integrity check failed (HTTP 422, `CONTENT_INTEGRITY`) — the stored bytes
    /// do not match the caller's declared hash (W2 PR 5). Distinct from `Conflict` because it is **not**
    /// resumable: the caller must discard the poisoned resource and re-upload, not retry.
    #[error("content integrity check failed: {message}")]
    ContentIntegrity { message: String },

    /// A data-artifact write the system declined for reasons the caller can act on — the
    /// SQL wrapper's refusal vocabulary (a missing namespace, an unrecognized enforcement
    /// term) or the enforcing-shape verdict's per-violation detail. A `400` from the
    /// data-artifact write routes, deliberately **not** routed through [`Self::Server`]:
    /// before this variant the refusal surfaced as `Server { status: 500 }` with the
    /// generic internal body — a caller fault reported as a server fault, and the refusal
    /// teaching nothing. Discriminated by the wire `code` being
    /// [`temper_core::error::DATA_ARTIFACT_REFUSAL_CODE`], exactly as the 403 and 422 arms
    /// discriminate theirs — never by sniffing the message. Renders bare: the refusal's
    /// own words are the whole message.
    #[error("{message}")]
    DataArtifactRefusal { message: String },

    #[error("rate limited — retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },

    /// A write that waited on a row lock past the server's bound and was rolled back having
    /// applied nothing (HTTP 503, code [`temper_core::error::RESOURCE_BUSY_CODE`]). Distinct from
    /// [`Self::Server`] because it carries a promise a `500` does not: sending the same request
    /// again cannot double-apply. So `HttpClient` retries it for every method, unkeyed writes
    /// included, which it never does for a `Server` error.
    #[error("{message}")]
    ResourceBusy { message: String },

    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },

    /// A required cloud-configuration field (API URL, OAuth callback URL) is
    /// empty, or an endpoint is configured with a scheme this client refuses —
    /// plaintext `http` off the loopback interface, which would put a
    /// credential on the wire in the clear (see [`crate::endpoint`]). Surfaced
    /// before any network attempt so the user gets an actionable "run
    /// `temper init`" or "use https" message instead of a cryptic reqwest
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
