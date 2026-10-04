//! Resource re-block tool — one bounded, resumable corpus re-blocking step.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G4 — the last direct
//! cluster): the tool forwards to `POST /api/resources/reblock`
//! (`temper-api/src/handlers/reblock.rs`) as a per-request temper-client relay built
//! from the request's `Parts` and never touches the `DbBackend` directly. The wire
//! request is built straight from the input, per the register's decided pattern;
//! the route's per-row gate train and the deployment-wide `all` arm's system-admin
//! gate live in the shared backend, unchanged.
//!
//! The `scope=context` arm addresses its context by ref (`@me/…`, `+team/…`); the wire
//! request's `ReblockScope::Context` carries only a UUID, so the ref resolves first through
//! the shared `cognitive_maps::context_anchor` — itself a relay to
//! `GET /api/contexts/resolve` (teardown). Its faces, and its one declared delta, are the
//! context orientation tools' own: one anchor, one dialect.
//!
//! # Declared parity deltas (per the register's G3c delta format)
//!
//! - **NotFound prefix drops**: the direct map prefixed `{action}: ` on the missing
//!   resource/context arms; the door's `ClientError::NotFound` carries the server's
//!   own sentence, and the door does not re-apply a prefix the direct tool applied.
//!   Kind (`invalid_params`) and gate identical.
//! - **Conflict arm added**: the direct map had no Conflict arm (it fell to the
//!   internal_error catch-all, unreachable in practice); the door's 409 renders
//!   `invalid_params` with the server's sentence, prefix and `Conflict: ` label
//!   stripped by the shared `api_error_cause` strip.
//!
//! Every other arm maps arm-for-arm: `BadRequest` is `invalid_params` bare, the
//! system-admin `Forbidden` arm keeps the tool's own system-administrator sentence
//! under INVALID_REQUEST (the deployment-wide `all` scope's gate — the direct
//! binding's own voice, preserved per the G3d delta-5 precedent), and the
//! deployment's refusal kinds ride `AcrossAuth`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_client::error::ClientError;
use temper_core::types::ids::ResourceId;
use temper_core::types::reblock::{ReblockScope, DEFAULT_REBLOCK_LIMIT};
use uuid::Uuid;

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};
use crate::tools::cognitive_maps::context_anchor;

// ── Input structs ──────────────────────────────────────────────────────────────

/// Which arm of the adoption walk this invocation covers.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ReblockTarget {
    /// Exactly one resource, by ref.
    Resource,
    /// Every candidate homed in one context, enumerated through your own visibility.
    Context,
    /// Deployment-wide. Requires system-administrator standing.
    All,
}

/// MCP input for resource_reblock — one bounded, resumable re-blocking step.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceReblockInput {
    /// What this invocation covers — exactly one arm.
    pub scope: ReblockTarget,
    /// Resource ref (a UUID or the decorated `slug-<uuid>` form). Required when `scope` is
    /// `resource`; ignored otherwise.
    #[serde(default)]
    pub resource: Option<String>,
    /// Context ref (a UUID, `@owner/slug`, or `+team/slug`). Required when `scope` is
    /// `context`; ignored otherwise.
    #[serde(default)]
    pub context: Option<String>,
    /// Survey instead of act: classify every candidate without changing anything. Survey
    /// first, then run with false, then survey again to verify.
    #[serde(default)]
    pub dry_run: bool,
    /// How many candidates one call considers. A conservative default applies when omitted —
    /// a convenience, never a correctness bound; the receipt's cursor keeps a walk resumable.
    #[serde(default)]
    pub limit: Option<i64>,
    /// Resume key from the previous receipt — only candidates after it are considered.
    #[serde(default)]
    pub after_id: Option<Uuid>,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Client errors into rmcp errors, in the G3c/G3d mapping idiom (see the module
/// header for the declared deltas). `map_err` here is the direct binding's mapper
/// rewired to the client's typed refusals.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::ResourceErased { id } => crate::tools::resources::erased_error(id),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: {message}"),
            None,
        ),
        // The bare `Forbidden` that escapes the backend command is ONLY the deployment-wide
        // `all` arm's system-admin gate (`reblock_resources`'s scope seam) — per-row gate
        // refusals arrive as `Denied` receipt rows inside the batch, never as this error. The
        // refusal keeps the direct binding's own sentence: it names the actual gate, because
        // the sibling text ("cannot modify this resource") names a resource no scope addressed
        // and sends the agent on a false single-resource repair path.
        ClientError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!(
                "{action}: the deployment-wide `all` scope requires system-administrator \
                 standing — the `resource` and `context` scopes ride ordinary visibility \
                 instead and never refuse on reach alone"
            ),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

/// Run one bounded, resumable re-blocking step over the tool's scope arm.
///
/// CLI equivalent: `temper admin reblock --resource|--context|--all`.
pub async fn resource_reblock(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ResourceReblockInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    // The tool's discriminator shape maps onto the wire request's scope enum; the per-arm
    // required fields are refused here, at the door, so an agent repairs in one round trip.
    let scope = match input.scope {
        ReblockTarget::Resource => {
            let r = input.resource.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "scope=resource requires `resource`".to_string(),
                    None,
                )
            })?;
            let resource: ResourceId = temper_workflow::operations::parse_ref(&r)
                .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
            ReblockScope::Resource(Uuid::from(resource))
        }
        ReblockTarget::Context => {
            let c = input.context.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "scope=context requires `context`".to_string(),
                    None,
                )
            })?;
            ReblockScope::Context(context_anchor(svc, parts, &c).await?)
        }
        ReblockTarget::All => ReblockScope::All,
    };

    let request = temper_core::types::reblock::ReblockRequest {
        scope,
        dry_run: input.dry_run,
        limit: Some(input.limit.unwrap_or(DEFAULT_REBLOCK_LIMIT)),
        after_id: input.after_id,
    };

    let out = svc
        .relay_client(parts)?
        .admin()
        .reblock(&request)
        .await
        .across_auth(|e| map_err(e, "resource_reblock"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&out)),
    ]))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate the tool input schema the same way rmcp does at runtime
    /// (`SchemaSettings::draft2020_12`, ref-based generator) — the `blobs.rs` harness.
    /// Scalar enums emitted as `$ref` into `$defs` reach the Anthropic tool-use layer with
    /// no type signal and come back as `null`.
    fn rmcp_schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap()
    }

    /// A schema field must inline its string enum rather than reference it via `$ref` —
    /// the Anthropic $ref constraint. Two inline shapes are admissible: the flat
    /// `{"type":"string","enum":[…]}` form and the `oneOf`-of-string-consts form a locally
    /// `#[schemars(inline)]` enum produces. What may NOT pass is a `$ref`: the client sees
    /// no values and sends `null`.
    fn assert_inline_string_enum(field: &serde_json::Value, variants: &[&str]) {
        assert!(
            field.get("$ref").is_none(),
            "field must be inlined, not a $ref: {field}"
        );
        if let Some(got) = field.get("enum").and_then(|e| e.as_array()) {
            let type_ok = match field.get("type").and_then(|t| t.as_str()) {
                Some("string") => true,
                _ => field.get("type").and_then(|t| t.as_array()).is_some(),
            };
            assert!(
                type_ok,
                "flat enum must carry a string (or string|null) type: {field}"
            );
            let got: Vec<&str> = got.iter().filter_map(|v| v.as_str()).collect();
            assert_eq!(got, variants, "inline enum variants must match: {field}");
            return;
        }
        let one_of = field
            .get("oneOf")
            .and_then(|o| o.as_array())
            .unwrap_or_else(|| panic!("field must carry an inline string enum: {field}"));
        let got: Vec<&str> = one_of
            .iter()
            .map(|v| {
                assert_eq!(
                    v.get("type").and_then(|t| t.as_str()),
                    Some("string"),
                    "oneOf arm must be a string const: {field}"
                );
                v.get("const")
                    .and_then(|c| c.as_str())
                    .expect("oneOf arm carries a const")
            })
            .collect();
        assert_eq!(got, variants, "inline enum variants must match: {field}");
    }

    /// The tool input parses all three scope arms: one-target arms carry their ref, the
    /// deployment-wide arm needs nothing else.
    #[test]
    fn resource_reblock_input_deserializes_all_three_scope_arms() {
        let one: ResourceReblockInput = serde_json::from_value(serde_json::json!({
            "scope": "resource",
            "resource": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "dry_run": true
        }))
        .unwrap();
        assert!(matches!(one.scope, ReblockTarget::Resource));
        assert_eq!(
            one.resource.as_deref(),
            Some("019e84ab-26ba-7560-9d34-c60d74a9fbe2")
        );
        assert!(one.dry_run);

        let many: ResourceReblockInput = serde_json::from_value(serde_json::json!({
            "scope": "context",
            "context": "@me/temper",
            "limit": 25
        }))
        .unwrap();
        assert!(matches!(many.scope, ReblockTarget::Context));
        assert_eq!(many.context.as_deref(), Some("@me/temper"));
        assert_eq!(many.limit, Some(25));
        assert!(!many.dry_run, "dry_run defaults to act");

        let everywhere: ResourceReblockInput = serde_json::from_value(serde_json::json!({
            "scope": "all",
            "after_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe3"
        }))
        .unwrap();
        assert!(matches!(everywhere.scope, ReblockTarget::All));
        assert!(everywhere.resource.is_none() && everywhere.context.is_none());
        assert!(everywhere.after_id.is_some());
    }

    /// The scope enum must advertise its variants INLINE in the emitted input schema — a
    /// `$ref` enum reaches Anthropic tool-use as null (the known production failure).
    /// FAILS IF: the enum loses `#[schemars(inline)]` and drifts into `$defs`.
    #[test]
    fn resource_reblock_schema_inlines_its_scope_enum() {
        let schema = rmcp_schema_for::<ResourceReblockInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once enums are inlined: {schema}"
        );
        assert_inline_string_enum(
            &schema["properties"]["scope"],
            &["resource", "context", "all"],
        );
    }

    /// The only bare `Forbidden` that escapes the backend command is the deployment-wide `all`
    /// arm's `is_system_admin` gate (per-row gate refusals arrive as `Denied` receipt rows
    /// inside the batch — see `map_decline`) — so the mapper must name THAT gate. The borrowed
    /// sibling text ("cannot modify this resource") names a resource no scope addressed and
    /// sends the agent on a false single-resource repair path.
    /// FAILS IF: the `Forbidden` arm drifts back to the generic resource-modification wording.
    #[test]
    fn the_forbidden_mapper_names_the_system_administrator_gate() {
        let err = map_err(ClientError::Forbidden, "resource_reblock");
        assert!(
            err.message.contains("system-administrator"),
            "the refusal must name system-administrator standing, got: {}",
            err.message
        );
        assert!(
            !err.message.contains("cannot modify this resource"),
            "the refusal must not name a resource no scope addressed: {}",
            err.message
        );
    }
}
