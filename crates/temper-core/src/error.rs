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

/// The wire `error.code` a composition body the server could not read travels under — malformed
/// JSON, a value of the wrong type, an unknown `with` section, a body past the door's own limit. The
/// status is the reader's own (`400`, `413`, `415` or `422`); the message names what failed and
/// repeats at most a typo's worth of what the caller sent. A platform that refuses a large body
/// before the door reads it answers without this code.
///
/// **Not [`PLAN_REFUSED_CODE`].** A refused plan parsed and failed `validate`, and carries every
/// refusal at once; an unreadable one has no plan to collect refusals for. **Not `BAD_REQUEST`**
/// either, because the MCP door relays plans unread and needs the code to hand its caller the
/// API's own sentence as a caller error rather than a server fault.
///
/// Spelled here for the same reason as [`FORBIDDEN_DETAIL_CODE`] — the producer
/// (`temper-services`' `IntoResponse`) and the consumer (`temper-client`'s status mapper) name one
/// constant rather than two literals nothing checks.
pub const UNREADABLE_PLAN_CODE: &str = "UNREADABLE_PLAN";

/// The wire `error.code` a declined data-artifact write travels under — a `400` carrying the
/// refusal's own words: the SQL wrapper's vocabulary (a missing namespace, an unrecognized
/// enforcement term) or the enforcing-shape verdict's per-violation detail.
///
/// **A code of its own rather than `BAD_REQUEST`** — for the same reason as
/// [`PLAN_REFUSED_CODE`]: the client branches on the code to render the refusal bare and
/// typed, and reusing the generic code would force a message-text heuristic. The refusal is
/// a well-formed request the system says no to; the internal-error class is for server
/// faults, and mapping the former onto the latter is the laundering this code exists to
/// make un-necessary.
///
/// Spelled here for the same reason as its siblings — the producer (`temper-services`'
/// `IntoResponse`) and the consumer (`temper-client`'s status mapper) name one constant
/// rather than two literals nothing checks.
pub const DATA_ARTIFACT_REFUSAL_CODE: &str = "DATA_ARTIFACT_REFUSAL";

/// The wire `error.code` a read of an erased resource travels under — a `410` rendered only to a
/// caller who holds standing on the husk (`resource_husk_held_by`); every other caller gets the
/// uniform `404` an unknown id gets. Standing is decided at read time, not frozen at the act
/// (resource erasure spec D6), so it includes anyone who joined a granted team since.
///
/// **The spelling.** The resource erasure spec's `resource_erased` names the *signal*, not this
/// literal. The literal is upper snake, `RESOURCE_ERASED`, like every other code on the wire.
///
/// **A code of its own rather than `GONE`.** `GONE` is the folded-block `410`, whose row survives
/// as history and can still be addressed. A client branches on the code to tell "this resource was
/// erased" from "this block was folded"; reusing `GONE` would force it to sniff the message.
///
/// Spelled here for the same reason as [`FORBIDDEN_DETAIL_CODE`] — the producer
/// (`temper-services`' `IntoResponse`) and the consumer (the client, from build order 2c) name one
/// constant rather than two literals nothing checks.
pub const RESOURCE_ERASED_CODE: &str = "RESOURCE_ERASED";

/// The wire `error.code` an append or finalize on an ended ingest travels under — a `409` for
/// SQLSTATE `TF004`: the resource's ingest is `cancelled` or `abandoned`, and no append or
/// re-finalize can continue it.
///
/// **A code of its own rather than `CONFLICT`.** The other finalize `409`s (block count, body
/// merkle) are resumable: a client re-lists the landed blocks and appends the gap. This one is
/// not, and a resuming client branches on the code to drop its resume record and start a fresh
/// upload; reusing `CONFLICT` would force it to sniff the message.
///
/// Spelled here for the same reason as [`FORBIDDEN_DETAIL_CODE`] — the producer
/// (`temper-services`' `IntoResponse`) and the consumer (`temper-client`'s status mapper) name one
/// constant rather than two literals nothing checks.
pub const INGEST_ENDED_CODE: &str = "INGEST_ENDED";

/// The wire `error.code` a write travels under when it waited on a row lock past the write-side
/// bound (SQLSTATE `55P03`, `lock_not_available`): a `503` with `Retry-After`.
///
/// **What it promises.** The transaction rolled back, so nothing the request asked for was
/// applied, and the same request sent again is not a double-apply. That is the difference from an
/// `INTERNAL_ERROR` `500`, which carries no such promise.
///
/// **Why `503`.** Every shipped client classifies any 5xx as transient, so this changes no
/// client's behaviour (`409` would collide with "already exists"). None of them auto-retries an
/// unkeyed write: the caller decides.
///
/// Spelled here for the same reason as [`FORBIDDEN_DETAIL_CODE`].
pub const RESOURCE_BUSY_CODE: &str = "RESOURCE_BUSY";

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

    /// The addressed thing PERSISTS but is gone for this operation — a folded content block
    /// under write addressing (the defined-dangling-state design). Distinct from
    /// [`Self::NotFound`]: the row survives as history, and a silent 404 would read as "never
    /// existed". 410 on HTTP, named on every surface.
    #[error("{0}")]
    Gone(String),

    /// The addressed resource was ERASED (`kb_resources.erased_at` is set) and the caller holds
    /// standing on the husk. 410 on HTTP under [`RESOURCE_ERASED_CODE`]. Carries the id and
    /// nothing else: the message is fixed, so no title, hash, ingest state or erasure time can
    /// ride it. Distinct from [`Self::Gone`], which is a folded block, and produced only where the
    /// read would otherwise have been [`Self::NotFound`] — a caller without standing keeps that.
    #[error("resource {0} was erased")]
    ResourceErased(crate::types::ids::ResourceId),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),
    /// An append or finalize on an ingest that has ended (`cancelled` or `abandoned`, SQLSTATE
    /// `TF004`). A `409` like [`Self::Conflict`], but **not** resumable, so it travels the wire
    /// under [`INGEST_ENDED_CODE`]: a resuming client drops its resume record and starts a new
    /// upload rather than resuming into the same refusal.
    #[error("{0}")]
    IngestEnded(String),
    /// A finalize raw-bytes integrity check failed — the stored bytes do not match the caller's
    /// declared hash (W2 PR 5). Distinct from `Conflict` because it is **not** resumable: the caller
    /// (e.g. the CLI's segmented upload) must discard the poisoned resource and re-upload, not retry.
    #[error("{0}")]
    ContentIntegrity(String),

    /// A write that waited on a row lock past the write-side bound and was rolled back, having
    /// applied nothing. `503` with `Retry-After` under [`RESOURCE_BUSY_CODE`].
    #[error("the resource is busy; nothing was applied, retry the request")]
    ResourceBusy,

    /// A data-artifact write the system declined for reasons the caller can act on — the
    /// SQL wrapper's refusal vocabulary or the enforcing-shape verdict's per-violation
    /// detail. Travels the wire under [`DATA_ARTIFACT_REFUSAL_CODE`] so a client
    /// discriminates it by code, never by sniffing the message. 400 on HTTP: a
    /// well-formed request the system says no to — the internal-error class is for
    /// server faults, and the two must not blur.
    #[error("{0}")]
    DataArtifactRefusal(String),

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

/// The stable `error.code` string each [`TemperError`] variant maps to on the
/// CLI's structured stdout error payload.
///
/// Spelled here — alongside the variants it describes, and in the crate both
/// the CLI and the API depend on — rather than as a literal in `main.rs` and
/// nowhere else. The same pattern as [`FORBIDDEN_DETAIL_CODE`] and
/// [`PLAN_REFUSED_CODE`]: a wire string a caller branches on is a named
/// constant, not a spell.
///
/// `SystemAccessRequired` carries structured access-gate details that the
/// enriched renderer in `access_gate.rs` handles; it gets its own code so the
/// JSON error payload can distinguish it from a plain `Forbidden`.
impl TemperError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::VaultNotFound => "vault-not-found",
            Self::Config(_) => "config",
            Self::Vault(_) => "vault",
            Self::Project(_) => "project",
            Self::Embedding(_) => "embedding",
            Self::Index(_) => "index",
            Self::Io(_) => "io",
            Self::Yaml(_) => "yaml",
            Self::Json(_) => "json",
            Self::Toml(_) => "toml",
            Self::Extraction(_) => "extraction",
            Self::Api(_) => "api",
            Self::Network(_) => "network",
            Self::NotFound(_) => "not-found",
            Self::Gone(_) => "gone",
            Self::ResourceErased(_) => RESOURCE_ERASED_CODE,
            Self::BadRequest(_) => "bad-request",
            Self::Conflict(_) => "conflict",
            Self::IngestEnded(_) => INGEST_ENDED_CODE,
            Self::ContentIntegrity(_) => "content-integrity",
            Self::ResourceBusy => RESOURCE_BUSY_CODE,
            Self::DataArtifactRefusal(_) => DATA_ARTIFACT_REFUSAL_CODE,
            Self::Forbidden => "forbidden",
            Self::ForbiddenDetail(_) => FORBIDDEN_DETAIL_CODE,
            Self::Unauthorized(_) => "unauthorized",
            Self::SystemAccessRequired(_) => "system-access-required",
        }
    }
}
