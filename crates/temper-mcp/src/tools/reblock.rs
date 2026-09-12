//! Resource re-block tool — one bounded, resumable corpus re-blocking step.
//!
//! Mirrors the HTTP endpoint `POST /api/resources/reblock` (`temper-api/src/handlers/reblock.rs`)
//! and dispatches through `DbBackend` — the same write path the HTTP handler uses, so the
//! per-resource gate train and the deployment-wide `all` arm's system-admin gate live in the
//! backend command, not here. The tool input is scope-discriminated (the unified `facet_set`
//! naming shape): one `scope` discriminator plus the per-arm ref fields, mapped onto the wire
//! request's [`ReblockScope`].

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_core::context_ref::parse_context_ref;
use temper_core::error::TemperError;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::reblock::{ReblockScope, DEFAULT_REBLOCK_LIMIT};
use temper_services::backend::DbBackend;
use temper_services::services::context_service::resolve_context_ref;
use temper_workflow::operations::{Backend, ReblockResources, Surface};
use uuid::Uuid;

use crate::service::TemperMcpService;

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

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("{action}: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::ForbiddenDetail(msg) => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: {msg}"),
            None,
        ),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot modify this resource"),
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
    input: ResourceReblockInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

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
            let cref = parse_context_ref(&c).map_err(|e| {
                rmcp::ErrorData::invalid_params(format!("invalid context ref: {e}"), None)
            })?;
            let context = resolve_context_ref(pool, profile_id, &cref)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("context not found: {e}"), None)
                })?;
            ReblockScope::Context(Uuid::from(context))
        }
        ReblockTarget::All => ReblockScope::All,
    };

    let cmd = ReblockResources {
        scope,
        dry_run: input.dry_run,
        limit: input.limit.unwrap_or(DEFAULT_REBLOCK_LIMIT),
        after_id: input.after_id,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .reblock_resources(cmd)
        .await
        .map_err(|e| map_err(e, "resource_reblock"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
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
}
