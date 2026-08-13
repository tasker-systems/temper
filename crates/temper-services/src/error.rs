use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

use temper_core::types::error_details::{ErrorDetails, PlanRefusalDetails};
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
    #[error("Conflict: {0}")]
    Conflict(String),
    /// Finalize's raw-bytes integrity check failed — the stored bytes do not hash to the caller's
    /// declared `expected_content_hash` (W2 PR 5). A 422 with a distinct code (`CONTENT_INTEGRITY`)
    /// because, unlike a block-count/merkle `Conflict`, this is **not resumable**: the committed bytes
    /// are wrong and `block_append` refuses to overwrite a seq, so the caller must discard + re-upload.
    #[error("Content integrity check failed: {0}")]
    ContentIntegrity(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    error: ErrorDetail,
}

impl ErrorBody {
    /// Build a typed error body with no `details` payload — the shape used by
    /// both `ApiError::into_response` and the router fallback handler (the latter
    /// lives in temper-api, so this constructor is `pub` across the crate boundary).
    pub fn new(code: &'static str, message: String) -> Self {
        Self {
            error: ErrorDetail {
                code,
                message,
                details: None,
            },
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorDetail {
    code: &'static str,
    message: String,
    /// Present on `SYSTEM_ACCESS_REQUIRED`, where it carries the typed access refusal, and on
    /// `PLAN_REFUSED`, where it carries every static refusal of a composition; absent on every
    /// other error.
    // Held as a `Value` because `IntoResponse` erases the variant before serializing, but declared
    // to the generators as what it actually is: an untyped `details` described nothing while
    // costing the SDKs their typed refusal.
    //
    // `[widened — 2026-08-13]` This was declared as the bare `SystemAccessDetails` under a note
    // saying "should a second variant ever carry details, this becomes a `oneOf` — widen it then,
    // deliberately." B1 is that second variant, and this is that widening. Which ARM a body carries
    // is told by `error.code`, never by sniffing the payload's shape — the two arms are
    // distinguishable by required field (see `ErrorDetails`), but a client that leans on that is
    // one all-optional arm away from silently misparsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<ErrorDetails>)]
    details: Option<serde_json::Value>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
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
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::ContentIntegrity(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "CONTENT_INTEGRITY")
            }
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let message = match &self {
            ApiError::SystemAccessRequired { .. } => {
                "This system requires approved access.".to_string()
            }
            other => other.to_string(),
        };
        let status_code = status.as_u16();

        match &self {
            ApiError::NotFound(_) => {
                tracing::debug!(status_code, error_code = code, %message, "not found");
            }
            ApiError::Conflict(_) => {
                tracing::info!(status_code, error_code = code, %message, "conflict");
            }
            ApiError::ContentIntegrity(_) => {
                tracing::warn!(status_code, error_code = code, %message, "content integrity");
            }
            ApiError::Unauthorized(_) | ApiError::Forbidden | ApiError::ForbiddenDetail(_) => {
                tracing::warn!(status_code, error_code = code, %message, "auth error");
            }
            ApiError::SystemAccessRequired { .. } => {
                tracing::info!(status_code, error_code = code, "system access required");
            }
            ApiError::BadRequest(_) => {
                tracing::warn!(status_code, error_code = code, %message, "bad request");
            }
            ApiError::PlanRefused { refusals } => {
                // The count, not the refusals themselves — a composition is caller-authored content
                // and its refusal details quote it back.
                tracing::warn!(
                    status_code,
                    error_code = code,
                    refusal_count = refusals.len(),
                    "plan refused"
                );
            }
            ApiError::Internal(_) => {
                tracing::error!(status_code, error_code = code, %message, "internal error");
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
                code,
                message,
                details: details_json,
            },
        };
        (status, axum::Json(body)).into_response()
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
                tracing::error!("Database error: {err}");
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
            ApiError::Conflict(s) => TemperError::Conflict(s),
            ApiError::ContentIntegrity(s) => TemperError::ContentIntegrity(s),
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
            TemperError::Forbidden => ApiError::Forbidden,
            TemperError::ForbiddenDetail(s) => ApiError::ForbiddenDetail(s),
            TemperError::Unauthorized(s) => ApiError::Unauthorized(s),
            TemperError::BadRequest(s) => ApiError::BadRequest(s),
            TemperError::Conflict(s) => ApiError::Conflict(s),
            TemperError::ContentIntegrity(s) => ApiError::ContentIntegrity(s),
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
}
