use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use temper_core::types::error_details::{ErrorDetails, PlanRefusalDetails};
use temper_core::types::ids::ResourceId;
use temper_core::types::query::validate::PlanRefusal;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Renders as the bare message, with no `Not found:` prefix — unlike `BadRequest` and
    /// `Conflict`, whose prefixes double up by the time the client re-renders them. The payload
    /// is the whole sentence (`goal <id> not found or not readable`), so a caller reads one
    /// clause rather than a label stacked on a label.
    ///
    /// **Where `NotFound` stands in for `Forbidden`, the message must not confirm existence.**
    /// Several gates return 404 deliberately so a probe cannot become an existence oracle over
    /// subjects the caller has no standing to see. Those sites render through
    /// `ScopedAuthority::denial` (`crate::authz`), which is static and argument-free and so
    /// *cannot* name the subject even by accident — see the note on that method.
    #[error("{0}")]
    NotFound(String),
    /// 410 Gone — the addressed thing PERSISTS but is gone for this operation: a folded
    /// content block under write addressing (the defined-dangling-state design). Distinct
    /// from [`Self::NotFound`] because the row survives as history; a silent 404 would read
    /// as "never existed", and the write face joins the read contract's named states.
    #[error("{0}")]
    Gone(String),
    /// 410 under [`temper_core::error::RESOURCE_ERASED_CODE`] — the addressed resource was
    /// erased, and the caller holds standing on the husk (`resource_husk_held_by`). Produced only
    /// where the read would otherwise be [`Self::NotFound`], so a caller without standing keeps
    /// the uniform 404. Standing is decided at read time, not frozen at the act (spec D6; see
    /// `substrate_read::erased_or`).
    ///
    /// Carries the id and nothing else. The message is fixed: no title (a sentinel anyway), no
    /// `body_hash`, no `ingest_state`, no `erased_at`. The sentence is core's
    /// ([`temper_core::error::TemperError::ResourceErased`]), rendered through it rather than
    /// restated: the client parses the id back out of it, so one literal owns it.
    #[error("{}", erased_sentence(.0))]
    ResourceErased(ResourceId),
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Forbidden")]
    Forbidden,
    /// A `403` that **names the capability it refused**, for gates that have established the caller
    /// already READS the subject. Renders `403` exactly as [`Self::Forbidden`] does, under the
    /// distinct code [`temper_core::error::FORBIDDEN_DETAIL_CODE`] so a client can tell a
    /// message-bearing refusal from the message-less one without sniffing the message text.
    ///
    /// See [`temper_core::error::TemperError::ForbiddenDetail`] for the disclosure rule and why
    /// [`Self::Forbidden`] stays the argument-free default. In-tree producer:
    /// `DbBackend::check_cogmap_authorable`.
    #[error("{0}")]
    ForbiddenDetail(String),
    #[error("System access required")]
    SystemAccessRequired {
        details: Box<temper_core::types::access_gate::SystemAccessDetails>,
    },
    #[error("Bad request: {0}")]
    BadRequest(String),
    /// A `400` that **names every static reason a composition will not run**, under the distinct
    /// code [`temper_core::error::PLAN_REFUSED_CODE`].
    ///
    /// Not a [`Self::BadRequest`] carrying a joined string: `validate` returns *"every refusal, not
    /// the first — a caller repairing a plan should see all of it in one round trip"*, and that
    /// property survives to the caller only if the transport keeps the list a list. The distinct
    /// code is what lets a client know a body carries refusals without sniffing `details`.
    #[error("Plan refused: {} refusal(s)", .refusals.len())]
    PlanRefused { refusals: Vec<PlanRefusal> },
    /// A composition body the door could not read, under the distinct code
    /// [`temper_core::error::UNREADABLE_PLAN_CODE`] and the reader's own status. The message is
    /// the producer's to bound: it carries serde's account of what failed, which can quote the
    /// caller's input.
    #[error("{message}")]
    UnreadablePlan { status: StatusCode, message: String },
    #[error("Conflict: {0}")]
    Conflict(String),
    /// An append or finalize on an ingest that has ended (`cancelled` or `abandoned`, SQLSTATE
    /// `TF004`). Renders `409` with the same `Conflict:` sentence [`Self::Conflict`] renders, under
    /// the distinct code [`temper_core::error::INGEST_ENDED_CODE`]: unlike a block-count/merkle
    /// conflict it is **not resumable**, and a resuming client branches on the code, never on the
    /// message.
    #[error("Conflict: {0}")]
    IngestEnded(String),
    /// Finalize's raw-bytes integrity check failed — the stored bytes do not hash to the caller's
    /// declared `expected_content_hash` (W2 PR 5). A 422 with a distinct code (`CONTENT_INTEGRITY`)
    /// because, unlike a block-count/merkle `Conflict`, this is **not resumable**: the committed bytes
    /// are wrong and `block_append` refuses to overwrite a seq, so the caller must discard + re-upload.
    #[error("Content integrity check failed: {0}")]
    ContentIntegrity(String),
    /// A data-artifact write the system declined for reasons the caller can act on — the SQL
    /// wrapper's refusal vocabulary or the enforcing-shape verdict's per-violation detail.
    /// 400 under [`temper_core::error::DATA_ARTIFACT_REFUSAL_CODE`]: the client discriminates
    /// by code, never by sniffing the message (the rule [`Self::PlanRefused`] and
    /// [`Self::ContentIntegrity`] already follow). The producer is the substrate's typed
    /// `DataArtifactRefusal`, downcast at the write seams — never a 500 envelope.
    #[error("{0}")]
    DataArtifactRefusal(String),
    /// A request exceeding a chosen rate bound (the rate-limit seam,
    /// `crate::rate_limit`). This is the refusal face's *"a well-formed request the
    /// system says no to, not an error"*: a 429 with the house structured body, not a
    /// bare status — and a `Retry-After` computed from the window, so a well-behaved
    /// caller backs off to the moment a retry can actually succeed. Authorized as an
    /// EXTEND by the seam design's A6.
    #[error("Too many requests: {message}")]
    TooManyRequests {
        message: String,
        retry_after_secs: i64,
    },
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    /// A 5xx whose body names the DOOR, never the failure (F9). The crate's own
    /// `From<sqlx::Error>` posture — log the full error, render a generic message —
    /// applied wherever the error text is third-party content: a provider bail carries
    /// the provider's response body, a database error the database's message, and an
    /// `anyhow` chain concatenates whatever sits underneath it. None of it belongs on
    /// the wire: the caller sees the generic internal-error message, and the context
    /// string is log-only — the log line's prefix that routes the failure to an operator.
    pub fn internal_scrubbed(context: &str, err: impl std::fmt::Display) -> Self {
        tracing::error!(context, error = %err, "internal error (scrubbed from the response)");
        ApiError::Internal(context.to_string())
    }
}

// The error envelope's wire shapes live in temper-core (every wire type does); re-exported here,
// beside the `ApiError` that renders them, the way `temper_substrate::ids` re-exports the ids.
pub use temper_core::types::error_details::{ErrorBody, ErrorDetail};

/// What a 5xx body tells the client, in place of the internal detail.
///
/// The detail is server material — it can quote SQL, upstream URLs, or filesystem
/// paths, and its length is whatever produced the failure. The full text goes to the
/// `tracing::error!` event (bounded, [`MAX_LOGGED_ERROR_BYTES`]); the client gets this.
const INTERNAL_CLIENT_MESSAGE: &str = "An internal error occurred";

/// Bound on any single error message written to a log event. Messages are
/// attacker-influenced input (a `BadRequest` quoting a request, an internal error
/// quoting an upstream response), so a cap is what keeps one event from being
/// whatever size the input was.
pub const MAX_LOGGED_ERROR_BYTES: usize = 2048;

/// [`ApiError::ResourceErased`]'s message: core's `TemperError::ResourceErased` sentence, so the
/// literal has one owner on both sides of the wire.
fn erased_sentence(id: &ResourceId) -> String {
    temper_core::error::TemperError::ResourceErased(*id).to_string()
}

/// `s`, truncated on a char boundary with an ellipsis when past [`MAX_LOGGED_ERROR_BYTES`].
///
/// Public because every surface that writes an attacker-influenced string into a log
/// event needs the same cap — e.g. the Slack link handler logging the IdP's redirect
/// error parameter, which anyone can craft to any length.
pub fn bounded(s: &str) -> std::borrow::Cow<'_, str> {
    if s.len() <= MAX_LOGGED_ERROR_BYTES {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut cut = MAX_LOGGED_ERROR_BYTES;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    std::borrow::Cow::Owned(format!("{}…", &s[..cut]))
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Gone(_) => (StatusCode::GONE, "GONE"),
            ApiError::ResourceErased(_) => {
                (StatusCode::GONE, temper_core::error::RESOURCE_ERASED_CODE)
            }
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::ForbiddenDetail(_) => (
                StatusCode::FORBIDDEN,
                temper_core::error::FORBIDDEN_DETAIL_CODE,
            ),
            ApiError::SystemAccessRequired { .. } => {
                (StatusCode::FORBIDDEN, "SYSTEM_ACCESS_REQUIRED")
            }
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::PlanRefused { .. } => (
                StatusCode::BAD_REQUEST,
                temper_core::error::PLAN_REFUSED_CODE,
            ),
            ApiError::UnreadablePlan { status, .. } => {
                (*status, temper_core::error::UNREADABLE_PLAN_CODE)
            }
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::IngestEnded(_) => {
                (StatusCode::CONFLICT, temper_core::error::INGEST_ENDED_CODE)
            }
            ApiError::ContentIntegrity(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "CONTENT_INTEGRITY")
            }
            ApiError::DataArtifactRefusal(_) => (
                StatusCode::BAD_REQUEST,
                temper_core::error::DATA_ARTIFACT_REFUSAL_CODE,
            ),
            ApiError::TooManyRequests { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS")
            }
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let message = match &self {
            ApiError::SystemAccessRequired { .. } => {
                "This system requires approved access.".to_string()
            }
            // The generic string, never the detail — see `INTERNAL_CLIENT_MESSAGE`.
            ApiError::Internal(_) => INTERNAL_CLIENT_MESSAGE.to_string(),
            other => other.to_string(),
        };
        let status_code = status.as_u16();

        match &self {
            ApiError::NotFound(_) => {
                tracing::debug!(status_code, error_code = code, message = %bounded(&message), "not found");
            }
            ApiError::Conflict(_) | ApiError::IngestEnded(_) => {
                tracing::info!(status_code, error_code = code, message = %bounded(&message), "conflict");
            }
            ApiError::ContentIntegrity(_) => {
                tracing::warn!(status_code, error_code = code, message = %bounded(&message), "content integrity");
            }
            // Info, not warn: a 429 is the system working as configured — pressure the
            // operator chose to bound — not an instance fault. The retry value is the
            // actionable half; the message is caller-shaped text and quotes nothing
            // sensitive by construction (it names a route or a door).
            ApiError::TooManyRequests {
                retry_after_secs, ..
            } => {
                tracing::info!(
                    status_code,
                    error_code = code,
                    retry_after_secs,
                    message = %bounded(&message),
                    "rate limited"
                );
            }
            ApiError::Unauthorized(_) | ApiError::Forbidden | ApiError::ForbiddenDetail(_) => {
                tracing::warn!(status_code, error_code = code, message = %bounded(&message), "auth error");
            }
            ApiError::SystemAccessRequired { .. } => {
                tracing::info!(status_code, error_code = code, "system access required");
            }
            ApiError::BadRequest(_) => {
                tracing::warn!(status_code, error_code = code, message = %bounded(&message), "bad request");
            }
            // The code and status only, never the message: it is serde's account of a composition,
            // which is caller-authored content and can quote it — the `PlanRefused` rule below.
            ApiError::UnreadablePlan { .. } => {
                tracing::warn!(status_code, error_code = code, "unreadable plan");
            }
            ApiError::DataArtifactRefusal(_) => {
                tracing::warn!(status_code, error_code = code, message = %bounded(&message), "data artifact refused");
            }
            ApiError::Gone(_) => {
                tracing::debug!(status_code, error_code = code, message = %bounded(&message), "gone (folded address)");
            }
            ApiError::ResourceErased(_) => {
                tracing::debug!(status_code, error_code = code, message = %bounded(&message), "gone (erased resource)");
            }
            ApiError::PlanRefused { refusals } => {
                // The count and the REASONS, never the refusals themselves — a composition is
                // caller-authored content and `PlanRefusal::detail` quotes it back. `reason` is a
                // closed vocabulary this crate raises, so it carries no caller bytes.
                //
                // `[added — 2026-08-28, found in review]` The count alone made every refusal look
                // alike: a caller repeatedly tripping `TooManyIds` on a published ceiling was
                // indistinguishable from one sending a cyclic plan, so the operator could not tell
                // a mis-sized cap from a malformed request. Declared holes this does NOT close:
                // a client that enforces the published ceiling locally never reaches this door at
                // all, and a refusal is a 400 — `otel.status_code` is set to ERROR only for 5xx, so
                // this is greppable and not alertable.
                //
                // Debug, not the serde name: the wire spelling lives in one `rename_all` and
                // re-deriving it here would be a second copy free to drift from it.
                tracing::warn!(
                    status_code,
                    error_code = code,
                    refusal_count = refusals.len(),
                    reasons = ?refusals.iter().map(|r| &r.reason).collect::<Vec<_>>(),
                    "plan refused"
                );
            }
            ApiError::Internal(detail) => {
                // `message` above is now the generic client string; the log keeps the
                // bounded detail, which is server material the body must not carry.
                tracing::error!(
                    status_code,
                    error_code = code,
                    detail = %bounded(detail),
                    "internal error"
                );
            }
        }

        // Two named arms and a catch-all, deliberately: widening the `_` into something clever is
        // what would make a third details-carrying variant invisible when it arrives.
        let details_json = match &self {
            ApiError::SystemAccessRequired { details } => Some(
                serde_json::to_value(ErrorDetails::SystemAccess(details.clone()))
                    .unwrap_or_default(),
            ),
            ApiError::PlanRefused { refusals } => Some(
                serde_json::to_value(ErrorDetails::PlanRefusals(PlanRefusalDetails {
                    refusals: refusals.clone(),
                }))
                .unwrap_or_default(),
            ),
            _ => None,
        };

        let body = ErrorBody {
            error: ErrorDetail {
                code: code.to_string(),
                message,
                details: details_json,
            },
        };
        let mut response = (status, axum::Json(body)).into_response();

        // Retry-After rides only the 429 — it is that arm's semantics, and computing it
        // was the refusal's whole point. `to_string` of an i64 is a valid header value,
        // so the insert cannot fail; the match guards the semantics, not the bytes.
        if let ApiError::TooManyRequests {
            retry_after_secs, ..
        } = &self
        {
            if let Ok(value) = axum::http::HeaderValue::from_str(&retry_after_secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }

        response
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => ApiError::NotFound("not found".to_string()),
            sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                ApiError::Conflict("Resource already exists".to_string())
            }
            _ => {
                // Postgres embeds the offending value in several error classes
                // (`invalid input syntax for type uuid: "<value>"`), and this codebase
                // binds caller strings against `::uuid` casts — so the raw text is
                // caller-chosen content at caller-chosen length. Bounded like every
                // other error event; the client body is the generic string regardless.
                let err_text = err.to_string();
                tracing::error!(error = %bounded(&err_text), "database error");
                ApiError::Internal("An internal error occurred".to_string())
            }
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::BadRequest(format!("Invalid JSON: {err}"))
    }
}

impl From<ApiError> for temper_core::error::TemperError {
    fn from(err: ApiError) -> Self {
        use temper_core::error::{CliAccessDetails, TemperError};
        match err {
            ApiError::NotFound(s) => TemperError::NotFound(s),
            ApiError::Gone(s) => TemperError::Gone(s),
            ApiError::ResourceErased(id) => TemperError::ResourceErased(id),
            ApiError::Forbidden => TemperError::Forbidden,
            ApiError::ForbiddenDetail(s) => TemperError::ForbiddenDetail(s),
            ApiError::Unauthorized(s) => TemperError::Unauthorized(s),
            ApiError::BadRequest(s) => TemperError::BadRequest(s),
            // Degrades to the joined text rather than earning a `TemperError` arm of its own.
            // This conversion is the server-side DbBackend → CLI-shaped-error path, which a route
            // refusal never travels: `POST /api/query` renders `PlanRefused` straight to HTTP, and
            // the CLI meets the refusal list as a `ClientError` parsed back off the wire. Adding an
            // arm here would be a second, colder representation of the same list with no producer.
            ApiError::PlanRefused { refusals } => TemperError::BadRequest(
                refusals
                    .iter()
                    .map(|r| r.detail.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            // Degrades to BadRequest text for the same reason as `PlanRefused` above: the
            // extractor renders it straight to HTTP, and no server-side path converts it.
            ApiError::UnreadablePlan { message, .. } => TemperError::BadRequest(message),
            ApiError::Conflict(s) => TemperError::Conflict(s),
            ApiError::IngestEnded(s) => TemperError::IngestEnded(s),
            ApiError::ContentIntegrity(s) => TemperError::ContentIntegrity(s),
            ApiError::DataArtifactRefusal(s) => TemperError::DataArtifactRefusal(s),
            // Degrades to BadRequest text rather than earning a `TemperError` arm of its
            // own — the CLI renders errors as text, has no status to preserve, and the
            // retry value is exactly what the caller needs next, so it rides along. Same
            // shape as the `PlanRefused` degradation above and for the same reason.
            ApiError::TooManyRequests {
                message,
                retry_after_secs,
            } => TemperError::BadRequest(format!("{message} (retry after {retry_after_secs}s)")),
            ApiError::Internal(s) => TemperError::Api(format!("internal: {s}")),
            ApiError::SystemAccessRequired { details } => {
                TemperError::SystemAccessRequired(Box::new(CliAccessDetails {
                    email: details.email,
                    display_name: details.display_name,
                    refusal: Some(details.refusal),
                    request_url: details.request_url,
                    cli_command: details.cli_command,
                }))
            }
        }
    }
}

impl From<temper_core::error::TemperError> for ApiError {
    fn from(err: temper_core::error::TemperError) -> Self {
        use temper_core::error::TemperError;
        use temper_core::types::access_gate::SystemAccessDetails;

        match err {
            // Clean cases that mirror the inbound conversion
            TemperError::NotFound(s) => ApiError::NotFound(s),
            TemperError::Gone(s) => ApiError::Gone(s),
            TemperError::ResourceErased(id) => ApiError::ResourceErased(id),
            TemperError::Forbidden => ApiError::Forbidden,
            TemperError::ForbiddenDetail(s) => ApiError::ForbiddenDetail(s),
            TemperError::Unauthorized(s) => ApiError::Unauthorized(s),
            TemperError::BadRequest(s) => ApiError::BadRequest(s),
            TemperError::Conflict(s) => ApiError::Conflict(s),
            TemperError::IngestEnded(s) => ApiError::IngestEnded(s),
            TemperError::ContentIntegrity(s) => ApiError::ContentIntegrity(s),
            TemperError::DataArtifactRefusal(s) => ApiError::DataArtifactRefusal(s),
            TemperError::Api(s) => ApiError::Internal(s),
            TemperError::SystemAccessRequired(details) => {
                ApiError::SystemAccessRequired {
                    details: Box::new(SystemAccessDetails {
                        email: details.email,
                        display_name: details.display_name,
                        // An older server may have sent no typed refusal; default to the generic
                        // "no standing" denial when reconstructing the server-side shape.
                        refusal: details
                            .refusal
                            .unwrap_or(temper_principal::Refusal::NoStanding),
                        request_url: details.request_url,
                        cli_command: details.cli_command,
                    }),
                }
            }

            // CLI-facing variants that shouldn't normally bubble out of a server-side DbBackend
            TemperError::VaultNotFound => ApiError::Internal("vault not found".into()),
            TemperError::Config(s) => ApiError::Internal(format!("config: {s}")),
            TemperError::Vault(s) => ApiError::Internal(format!("vault: {s}")),
            TemperError::Project(s) => ApiError::Internal(format!("project: {s}")),
            TemperError::Embedding(s) => ApiError::Internal(format!("embedding: {s}")),
            TemperError::Index(s) => ApiError::Internal(format!("index: {s}")),
            TemperError::Io(e) => ApiError::Internal(format!("io: {e}")),
            TemperError::Yaml(e) => ApiError::BadRequest(format!("yaml: {e}")),
            TemperError::Json(e) => ApiError::BadRequest(format!("json: {e}")),
            TemperError::Toml(e) => ApiError::BadRequest(format!("toml: {e}")),
            TemperError::Extraction(s) => ApiError::Internal(format!("extraction: {s}")),
            TemperError::Network(s) => ApiError::Internal(format!("network: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::error::TemperError;

    /// Render an `ApiError` the way axum will and hand back the status plus the parsed body, so a
    /// test asserts on the BYTES a client receives rather than on the variant that produced them.
    async fn rendered(err: ApiError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body collects");
        (
            status,
            serde_json::from_slice(&bytes).expect("body is JSON"),
        )
    }

    fn refusal(detail: &str) -> PlanRefusal {
        use temper_core::types::query::disposition::RefusalReason;
        PlanRefusal {
            stage: None,
            reason: RefusalReason::UnknownAct,
            detail: detail.to_string(),
        }
    }

    /// The property Task B1.1 exists to make expressible: **every** refusal reaches the caller, on a
    /// 400, under its own code. A single-refusal assertion would pass against a body that truncates.
    #[tokio::test]
    async fn a_refused_plan_renders_400_with_every_refusal_under_its_own_code() {
        let (status, body) = rendered(ApiError::PlanRefused {
            refusals: vec![refusal("first"), refusal("second"), refusal("third")],
        })
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["error"]["code"],
            temper_core::error::PLAN_REFUSED_CODE,
            "a refused plan must be distinguishable from a generic BAD_REQUEST by CODE — the \
             client keys on it rather than sniffing the body's shape"
        );

        let refusals = body["error"]["details"]["refusals"]
            .as_array()
            .expect("details.refusals is an array — the wire path the spec names");
        assert_eq!(
            refusals.len(),
            3,
            "the refusal list was truncated in transit"
        );
        let details: Vec<&str> = refusals
            .iter()
            .map(|r| r["detail"].as_str().expect("detail is a string"))
            .collect();
        assert_eq!(details, ["first", "second", "third"]);
    }

    /// The regression boundary from the plan's *Declared risk*: `ErrorDetail` is on every route in
    /// the project, so widening `details` into a `oneOf` must leave the shipped 403 body untouched
    /// — status, code, and every byte of `details`. Asserted, not assumed.
    #[tokio::test]
    async fn widening_details_left_the_system_access_403_byte_identical() {
        use temper_core::types::access_gate::SystemAccessDetails;
        use temper_principal::Refusal;

        let details = SystemAccessDetails {
            email: Some("a@b.c".into()),
            display_name: Some("A".into()),
            refusal: Refusal::NoStanding,
            request_url: Some("https://example.test/join".into()),
            cli_command: Some("temper auth request-access".into()),
        };
        let (status, body) = rendered(ApiError::SystemAccessRequired {
            details: Box::new(details.clone()),
        })
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"]["code"], "SYSTEM_ACCESS_REQUIRED");
        assert_eq!(
            body["error"]["details"],
            serde_json::to_value(&details).expect("details serialize"),
            "the `oneOf` moved the access-refusal payload a shipped client already parses"
        );
    }

    /// `details` stays absent — not `null` — on the arms that carry none. `skip_serializing_if` is
    /// what makes that true, and a `oneOf` whose null arm leaked would add a key to every error
    /// body in the project.
    #[tokio::test]
    async fn an_error_carrying_no_details_still_omits_the_key_entirely() {
        let (status, body) = rendered(ApiError::BadRequest("missing field".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
        assert!(
            body["error"].get("details").is_none(),
            "details must be absent, not null, on an error that carries none"
        );
    }

    #[test]
    fn a_refused_plan_degrades_to_bad_request_when_crossing_into_temper_error() {
        let t: TemperError = ApiError::PlanRefused {
            refusals: vec![refusal("first"), refusal("second")],
        }
        .into();
        match t {
            TemperError::BadRequest(s) => assert_eq!(s, "first; second"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn api_error_forbidden_maps_to_temper_forbidden() {
        let t: TemperError = ApiError::Forbidden.into();
        assert!(matches!(t, TemperError::Forbidden));
    }

    #[test]
    fn api_error_bad_request_carries_message() {
        let t: TemperError = ApiError::BadRequest("missing field".into()).into();
        match t {
            TemperError::BadRequest(s) => assert_eq!(s, "missing field"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn api_error_conflict_carries_message() {
        let t: TemperError = ApiError::Conflict("duplicate".into()).into();
        match t {
            TemperError::Conflict(s) => assert_eq!(s, "duplicate"),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn api_error_unauthorized_carries_message() {
        let t: TemperError = ApiError::Unauthorized("no token".into()).into();
        match t {
            TemperError::Unauthorized(s) => assert_eq!(s, "no token"),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn api_error_internal_maps_to_temper_api() {
        let t: TemperError = ApiError::Internal("oops".into()).into();
        match t {
            TemperError::Api(s) => assert!(s.contains("oops")),
            other => panic!("expected Api(_), got {other:?}"),
        }
    }

    #[test]
    fn api_error_system_access_required_preserves_field_set() {
        use temper_core::types::access_gate::SystemAccessDetails;
        use temper_principal::Refusal;
        let api = ApiError::SystemAccessRequired {
            details: Box::new(SystemAccessDetails {
                email: Some("a@b.co".into()),
                display_name: Some("A".into()),
                // A real refusal, not the retired sentinel: `Revoked` is the case the typed value
                // exists to distinguish from `Denied`.
                refusal: Refusal::Revoked,
                request_url: Some("https://x".into()),
                cli_command: Some("temper join".into()),
            }),
        };
        let t: TemperError = api.into();
        match t {
            TemperError::SystemAccessRequired(details) => {
                assert_eq!(details.email.as_deref(), Some("a@b.co"));
                assert_eq!(details.display_name.as_deref(), Some("A"));
                assert_eq!(details.refusal, Some(Refusal::Revoked));
                assert_eq!(details.request_url.as_deref(), Some("https://x"));
                assert_eq!(details.cli_command.as_deref(), Some("temper join"));
            }
            other => panic!("expected SystemAccessRequired, got {other:?}"),
        }
    }

    // Outbound conversion tests (TemperError -> ApiError)

    /// The message **survives** the mapping.
    ///
    /// This test previously asserted only `matches!(t, ApiError::NotFound)` — true of the unit
    /// variant, and true of nothing else worth knowing. It pinned the discard: the service layer
    /// wrote a message naming what was missing and the door dropped it, so every 404 rendered as
    /// the bare string `Not found`. Carrying the string is the contract now, and asserting the
    /// variant alone would not notice if it were dropped again.
    #[test]
    fn temper_error_not_found_carries_message() {
        let t: ApiError = TemperError::NotFound("item missing".into()).into();
        match t {
            ApiError::NotFound(s) => assert_eq!(s, "item missing"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// And it survives the *inbound* direction too, so a client-side round-trip does not quietly
    /// re-flatten what the outbound direction just preserved.
    #[test]
    fn api_error_not_found_carries_message_to_temper() {
        let t: TemperError = ApiError::NotFound("goal 42 not found".into()).into();
        match t {
            TemperError::NotFound(s) => assert_eq!(s, "goal 42 not found"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_forbidden_maps_to_api_forbidden() {
        let t: ApiError = TemperError::Forbidden.into();
        assert!(matches!(t, ApiError::Forbidden));
    }

    #[test]
    fn temper_error_bad_request_carries_message() {
        let a: ApiError = TemperError::BadRequest("missing field".into()).into();
        match a {
            ApiError::BadRequest(s) => assert_eq!(s, "missing field"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_conflict_carries_message() {
        let a: ApiError = TemperError::Conflict("duplicate key".into()).into();
        match a {
            ApiError::Conflict(s) => assert_eq!(s, "duplicate key"),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_unauthorized_carries_message() {
        let a: ApiError = TemperError::Unauthorized("invalid token".into()).into();
        match a {
            ApiError::Unauthorized(s) => assert_eq!(s, "invalid token"),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_api_maps_to_internal() {
        let a: ApiError = TemperError::Api("internal issue".into()).into();
        match a {
            ApiError::Internal(s) => assert_eq!(s, "internal issue"),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_system_access_required_round_trip() {
        use temper_core::error::CliAccessDetails;
        use temper_principal::Refusal;

        // A typed refusal round-trips cleanly through the CLI error chain.
        let details = CliAccessDetails {
            email: Some("test@example.com".into()),
            display_name: Some("Test User".into()),
            refusal: Some(Refusal::Requested),
            request_url: Some("https://example.com/join".into()),
            cli_command: Some("temper join-request".into()),
        };

        let t_err = TemperError::SystemAccessRequired(Box::new(details));
        let a: ApiError = t_err.into();

        match a {
            ApiError::SystemAccessRequired { details } => {
                assert_eq!(details.email.as_deref(), Some("test@example.com"));
                assert_eq!(details.display_name.as_deref(), Some("Test User"));
                assert_eq!(details.refusal, Refusal::Requested);
                assert_eq!(
                    details.request_url.as_deref(),
                    Some("https://example.com/join")
                );
                assert_eq!(details.cli_command.as_deref(), Some("temper join-request"));
            }
            other => panic!("expected SystemAccessRequired, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_yaml_maps_to_bad_request() {
        let yaml_err: serde_yaml::Error =
            serde_yaml::from_str::<serde_yaml::Value>("invalid: : :").unwrap_err();
        let a: ApiError = TemperError::Yaml(yaml_err).into();
        match a {
            ApiError::BadRequest(s) => assert!(s.starts_with("yaml: ")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn temper_error_vault_not_found_maps_to_internal() {
        let a: ApiError = TemperError::VaultNotFound.into();
        match a {
            ApiError::Internal(s) => assert!(s.contains("vault not found")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    /// An erased resource renders `410` under its own code, with the fixed message and no
    /// `details`. FAILS IF the arm falls back to `GONE` (a client could not tell an erasure from
    /// a folded block), or the message grows anything beyond the id.
    #[tokio::test]
    async fn an_erased_resource_renders_410_under_its_own_code_with_only_the_id() {
        let id = ResourceId::from(uuid::Uuid::now_v7());
        let (status, body) = rendered(ApiError::ResourceErased(id)).await;

        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            body["error"]["code"],
            temper_core::error::RESOURCE_ERASED_CODE
        );
        assert_eq!(
            body["error"]["message"],
            format!("resource {id} was erased")
        );
        assert!(
            body["error"].get("details").is_none(),
            "an erased resource carries no details: {body}"
        );
    }

    /// One sentence for the erasure on both sides of the wire: the API renders exactly core's,
    /// which the client parses the id back out of. FAILS IF `ApiError::ResourceErased` stops
    /// delegating and either literal is reworded.
    #[test]
    fn the_erased_variant_renders_exactly_as_cores() {
        let id = ResourceId::from(uuid::Uuid::now_v7());
        assert_eq!(
            ApiError::ResourceErased(id).to_string(),
            TemperError::ResourceErased(id).to_string()
        );
    }

    /// An ended ingest renders `409` under its own code, with the same `Conflict:` sentence a
    /// resumable conflict renders, after crossing `DbBackend` (TemperError → ApiError). FAILS IF the
    /// arm falls back to `CONFLICT` (a resuming client could not tell an ended ingest from a gap it
    /// can append), or the conversion drops the variant.
    #[tokio::test]
    async fn an_ended_ingest_renders_409_under_its_own_code() {
        let a: ApiError = TemperError::IngestEnded("the ingest has ended".to_string()).into();
        let (status, body) = rendered(a).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], temper_core::error::INGEST_ENDED_CODE);
        assert_eq!(body["error"]["message"], "Conflict: the ingest has ended");
        let (status, body) = rendered(ApiError::Conflict("a gap".to_string())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body["error"]["code"], "CONFLICT",
            "a resumable conflict keeps its code"
        );
    }

    /// Both conversions keep the variant, so a read that crosses `DbBackend` (ApiError →
    /// TemperError → ApiError) still renders `410 RESOURCE_ERASED`, not a `GONE` or a `500`.
    #[test]
    fn resource_erased_survives_the_round_trip_through_temper_error() {
        let id = ResourceId::from(uuid::Uuid::now_v7());
        let t: TemperError = ApiError::ResourceErased(id).into();
        assert!(matches!(t, TemperError::ResourceErased(got) if got == id));
        assert_eq!(t.code(), temper_core::error::RESOURCE_ERASED_CODE);
        let back: ApiError = t.into();
        assert!(matches!(back, ApiError::ResourceErased(got) if got == id));
    }

    /// The 5xx body is client-facing, and the internal detail is server material — SQL,
    /// upstream URLs, paths — whose length is whatever produced the failure. The body
    /// carries the fixed generic string; the full text stays in the `tracing::error!`
    /// event, bounded.
    #[tokio::test]
    async fn a_5xx_body_carries_the_generic_message_never_the_internal_detail() {
        let detail = format!(
            "extraction failed near /var/tmp/{}</usr/local/very/long",
            "x".repeat(64)
        );
        let (status, body) = rendered(ApiError::Internal(detail.clone())).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["message"], INTERNAL_CLIENT_MESSAGE);
        let echoed = serde_json::to_string(&body).expect("body reserializes");
        assert!(
            !echoed.contains("/var/tmp/"),
            "the internal detail leaked into the client body: {echoed}"
        );
    }

    /// The cap is what keeps one log event from being whatever size the input was.
    #[test]
    fn bounded_truncates_past_the_cap_on_a_char_boundary() {
        let short = "short message";
        assert!(matches!(bounded(short), std::borrow::Cow::Borrowed(_)));

        let long = "y".repeat(MAX_LOGGED_ERROR_BYTES * 3);
        let cut = bounded(&long);
        assert!(cut.len() <= MAX_LOGGED_ERROR_BYTES + '…'.len_utf8());
        assert!(cut.ends_with('…'));

        let multibyte = "é".repeat(MAX_LOGGED_ERROR_BYTES);
        let cut = bounded(&multibyte);
        assert!(
            cut.is_char_boundary(cut.len() - '…'.len_utf8()),
            "truncated mid-character: {cut}"
        );
    }
}
