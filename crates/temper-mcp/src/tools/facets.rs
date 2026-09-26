//! Facet tools — the typed-property family (set on resource/edge, read both, retract-by-row).
//!
//! Execution crosses the DEPLOYED API over the wire (beat G3c, the network door — the
//! fourth family to cross): every tool drives a per-request temper-client HTTP relay
//! built from the request's `Parts` and never touches the pool, the `DbBackend`, or a
//! service function directly. The routes are the ones the CLI calls — `POST /api/facets`,
//! `POST/GET/DELETE /api/relationships/{handle}/facets…`, `GET /api/resources/{id}/facets`
//! — each forwarded on the caller's own bearer. The tool input's `ActInput` maps STRAIGHT
//! THROUGH into each request's `act` — never defaulted, never dropped — so authorship and
//! correlation ride the write the caller made, and the `origin: Surface::Mcp` stamp the
//! direct binding put on each command is now the relay's planted carrier at the door.
//!
//! The consolidated dispatchers' refusals that name the INPUT SHAPE (a `property_key`
//! beside `target=resource`, a `target=resource` retract, a missing `resource`/`edge_handle`)
//! stay MCP-local, before any forward — they are naming refusals, not gate outcomes.
//!
//! # Declared parity deltas (the direct faces pinned by `ledger_graph_parity_test.rs`,
//! re-derived at the door and flipped there deliberately)
//!
//! - **Closed-invocation 409**: the direct `map_err` had no `Conflict` arm, so the act
//!   gate's non-open refusal fell to the `internal_error` catch-all. The wire's 409 is
//!   caller-actionable — the run the correlation claim names is closed, which no retry
//!   changes and no input edit repairs except naming an open run — so it renders
//!   `invalid_params` with the server's own sentence, the `contexts.rs::map_api_error`
//!   Conflict arm's precedent (`a taken slug is caller-fixable, not an internal error`).
//!   The suite pinned `internal_error` pre-swap and pins the door's face now.
//! - **NotFound prefixes**: the direct map prefixed every not-found with `{action}: `.
//!   Over the wire, `ClientError::NotFound` carries the server's own sentence, and the
//!   door does not re-apply prefixes the direct tool applied — the kind (`invalid_params`)
//!   and the gate are identical, per the resources family's precedent (`resources.rs`'s
//!   context-refusal arm). Only the prefix drops; every sentence the server speaks is
//!   carried through verbatim.
//! - **Garbage-ref parse face**: the direct binding routed `parse_ref`'s failure through
//!   the shared error map's catch-all, rendering it `internal_error`; the door's MCP-local
//!   parse callsites render it `invalid_params`. This is a RENDERING delta of a parse face
//!   — not a naming refusal (the refusal KIND changed, the pre-wire gate did not) — and is
//!   pinned by `a_garbage_ref_through_resource_facets_refuses_as_invalid_params`.
//!
//! Every other arm maps arm-for-arm: `ForbiddenDetail` speaks the gate's own sentence
//! under INVALID_REQUEST prefixed `{action}: ` (the direct face byte-for-byte), the terse
//! `Forbidden` arm keeps the tool's fixed "cannot modify this resource" sentence, and the
//! 400 face passes the server's sentence through bare — the direct binding's pass-through.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_client::error::ClientError;
use temper_core::types::authorship::ActInput;
use temper_core::types::facet_requests::{EdgeFacetSetRequest, FacetSetRequest};
use uuid::Uuid;

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

// ── Input structs ──────────────────────────────────────────────────────────────

/// MCP input for facet_set.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FacetSetInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub resource: String,
    /// The facet's typed value payload — an **object** of `key` → value marks; one property row
    /// per inner key. A map, not a bare `Value`, so the advertised schema says `object` and a
    /// scalar payload is rejected here rather than discovered as a database error (the steward
    /// runs of 2026-09 sent a string eight times across two ticks, each retry a full re-read).
    pub values: serde_json::Map<String, serde_json::Value>,
    /// Facet salience/confidence weight (0.0-1.0 by convention). Defaults to 1.0.
    pub weight: Option<f64>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Map a facet-path error onto an MCP error, arm-for-arm with the direct binding.
///
/// The two declared parity deltas (409, NotFound prefix) and the arm-for-arm
/// discipline are argued in the module doc. In short: `NotFound` speaks the server's
/// sentence un-prefixed; `ForbiddenDetail` keeps the gate's own sentence under
/// INVALID_REQUEST with the direct wrapper's `{action}: ` voice; `Forbidden` keeps the
/// terse fixed sentence; the 400 passes the server's sentence through bare.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        // The server's own sentence, un-prefixed: the direct binding's tool wrapped
        // each not-found with "{action}: ", a prefix the door does not re-apply —
        // the kind (invalid_params) and the gate are identical.
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        // The door's 409 is caller-actionable — the direct catch-all rendered it
        // internal_error (the declared delta).
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // A refusal that named the capability it withheld — carry the gate's own sentence. The
        // terse arm below stays exactly as it was, for the caller who may not even read the subject.
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: {message}"),
            None,
        ),
        ClientError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot modify this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

/// Set a facet (typed property) on a resource — the steward's facet act.
///
/// Forwards to `POST /api/facets` through the network door; the act rides the request
/// verbatim. CLI equivalent: `temper resource facet <ref> --values '<json>'`.
pub async fn facet_set(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: FacetSetInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    // MCP-local input shaping: the ref parse never touches the wire.
    let resource = temper_workflow::operations::parse_ref(&input.resource)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?
        .uuid();

    let request = FacetSetRequest {
        resource,
        values: input.values,
        weight: input.weight.unwrap_or(1.0),
        // Straight through: authorship and correlation the caller supplied are the
        // write's authorship and correlation — never defaulted, never dropped.
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let ack = client
        .facets()
        .set(&request)
        .await
        .across_auth(|e| map_err(e, "facet_set"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

/// MCP input for `edge_facet_set`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EdgeFacetSetInput {
    /// The relationship's edge handle (the UUID `edge_assert` returned).
    pub edge_handle: Uuid,
    /// The facet's typed value payload — an **object** of `key` → value marks; same constraint as
    /// [`FacetSetInput::values`]. With `property_key` set, this is instead the ONE row's value
    /// under that key (e.g. `{"endpoint": "target", "address": "<resource-uuid>#<block-uuid>"}`
    /// for `anchored-at`).
    pub values: serde_json::Map<String, serde_json::Value>,
    /// Optional property key for a keyed single-row write (e.g. `anchored-at`): asserts `values`
    /// as ONE row under this key instead of the clustering `facet` verb. Omitted, the write is
    /// an ordinary facet.
    #[serde(default)]
    pub property_key: Option<String>,
    /// Facet salience/confidence weight (0.0-1.0 by convention). Defaults to 1.0.
    pub weight: Option<f64>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for `edge_facets` (read).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EdgeFacetsInput {
    /// The relationship's edge handle.
    pub edge_handle: Uuid,
}

/// MCP input for `resource_facets` (read).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceFacetsInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub resource: String,
}

/// Set a facet whose owner is an **edge** — a qualifier on a relationship rather than on a thing.
///
/// Forwards to `POST /api/relationships/{edge_handle}/facets` through the network door.
/// The edge is addressed by its handle, not a resource ref, and the server authorizes the
/// write through the edge's own mutability clauses (its source resource plus container-write
/// on its home) rather than through `can_modify_resource`.
///
/// CLI equivalent: `temper edge facet <edge-handle> --values '<json>'`.
pub async fn edge_facet_set(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: EdgeFacetSetInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let request = EdgeFacetSetRequest {
        values: input.values,
        property_key: input.property_key,
        weight: input.weight.unwrap_or(1.0),
        // Straight through: never defaulted, never dropped.
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let ack = client
        .facets()
        .set_on_edge(input.edge_handle, &request)
        .await
        .across_auth(|e| map_err(e, "edge_facet_set"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

/// Read the live facets of one edge, through the network door.
pub async fn edge_facets(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: EdgeFacetsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;
    let out = client
        .facets()
        .list_for_edge(input.edge_handle)
        .await
        .across_auth(|e| map_err(e, "edge_facets"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&out)),
    ]))
}

/// Read the live facets of one resource, through the network door.
///
/// The faithful view: one entry per live row, each with its weight and its author. `get_resource`
/// carries a facet inside `open_meta` collapsed to a single newest-wins value with the weight
/// discarded, so it cannot answer *"did my assert land"* when a key was asserted more than once.
///
/// CLI equivalent: `temper resource facets <ref>`.
pub async fn resource_facets(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ResourceFacetsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    // MCP-local input shaping: the ref parse never touches the wire.
    let resource = temper_workflow::operations::parse_ref(&input.resource)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?
        .uuid();

    let client = svc.relay_client(parts)?;
    let out = client
        .facets()
        .list_for_resource(resource)
        .await
        .across_auth(|e| map_err(e, "resource_facets"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&out)),
    ]))
}

// ── Consolidated write tool (2→1) ─────────────────────────────────────────────

/// The facet target — a resource or a relationship (edge).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum FacetTarget {
    /// Set a facet on a resource (node).
    Resource,
    /// Set a facet on a relationship (edge).
    Edge,
}

/// Consolidated facet-set tool — one write tool with a `target` discriminator.
///
/// Collapses `facet_set` (resource) and `edge_facet_set` (edge) into a single MCP tool.
/// The `target` field selects whether to set the facet on a resource or a relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FacetSetUnifiedInput {
    /// Whether to set the facet on a resource or a relationship.
    pub target: FacetTarget,
    /// Resource ref (UUID or `slug-<uuid>`). Required when `target` is `resource`; ignored otherwise.
    #[serde(default)]
    pub resource: Option<String>,
    /// The relationship's edge handle (UUID from `assert_relationship`). Required when `target` is `edge`; ignored otherwise.
    #[serde(default)]
    pub edge_handle: Option<Uuid>,
    /// The facet's typed value payload — an **object** of `key` → value marks; same constraint as
    /// [`FacetSetInput::values`]. With `property_key` set, this is instead the ONE row's value
    /// under that key.
    pub values: serde_json::Map<String, serde_json::Value>,
    /// Optional property key for a keyed single-row write (e.g. `anchored-at`); `target=edge`
    /// only — a keyed property row qualifies a relationship. Omitted, the write is an ordinary
    /// facet.
    #[serde(default)]
    pub property_key: Option<String>,
    /// Facet salience/confidence weight (0.0-1.0 by convention). Defaults to 1.0.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// Dispatch the consolidated facet-set tool.
pub async fn facet_set_unified(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: FacetSetUnifiedInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.target {
        FacetTarget::Resource => {
            if input.property_key.is_some() {
                return Err(rmcp::ErrorData::invalid_params(
                    "property_key applies to target=edge only — a keyed property row \
                     qualifies a relationship"
                        .to_string(),
                    None,
                ));
            }
            let resource = input.resource.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "target=resource requires `resource`".to_string(),
                    None,
                )
            })?;
            facet_set(
                svc,
                parts,
                FacetSetInput {
                    resource,
                    values: input.values,
                    weight: input.weight,
                    act: input.act,
                },
            )
            .await
        }
        FacetTarget::Edge => {
            let edge_handle = input.edge_handle.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "target=edge requires `edge_handle`".to_string(),
                    None,
                )
            })?;
            edge_facet_set(
                svc,
                parts,
                EdgeFacetSetInput {
                    edge_handle,
                    values: input.values,
                    property_key: input.property_key,
                    weight: input.weight,
                    act: input.act,
                },
            )
            .await
        }
    }
}

// ── Consolidated read tool (2→1) ───────────────────────────────────────────────

/// The facet-read target — a resource or a relationship (edge).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum FacetsReadTarget {
    /// Read facets of a resource (node).
    Resource,
    /// Read facets of a relationship (edge).
    Edge,
}

/// Consolidated facets-read tool — one read tool with a `target` discriminator.
///
/// Collapses `resource_facets` and `edge_facets` into a single MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FacetsReadInput {
    /// Whether to read facets from a resource or a relationship.
    pub target: FacetsReadTarget,
    /// Resource ref (UUID or `slug-<uuid>`). Required when `target` is `resource`; ignored otherwise.
    #[serde(default)]
    pub resource: Option<String>,
    /// The relationship's edge handle. Required when `target` is `edge`; ignored otherwise.
    #[serde(default)]
    pub edge_handle: Option<Uuid>,
}

/// Dispatch the consolidated facets-read tool.
pub async fn facets_read(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: FacetsReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.target {
        FacetsReadTarget::Resource => {
            let resource = input.resource.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "target=resource requires `resource`".to_string(),
                    None,
                )
            })?;
            resource_facets(svc, parts, ResourceFacetsInput { resource }).await
        }
        FacetsReadTarget::Edge => {
            let edge_handle = input.edge_handle.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "target=edge requires `edge_handle`".to_string(),
                    None,
                )
            })?;
            edge_facets(svc, parts, EdgeFacetsInput { edge_handle }).await
        }
    }
}

// ── Consolidated retract tool ──────────────────────────────────────────────────

/// The facet-retract target — a resource or a relationship (edge).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum FacetRetractTarget {
    /// Retract a facet from a resource — not supported; refused with the reason.
    Resource,
    /// Retract a facet from a relationship (edge).
    Edge,
}

/// Consolidated facet-retract tool — the correction verb, with the same `target`
/// discriminator as the set/read tools.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FacetRetractInput {
    /// Whether to retract a facet from a resource or a relationship.
    pub target: FacetRetractTarget,
    /// Resource ref (UUID or `slug-<uuid>`). `target=resource` is refused — see the tool description.
    #[serde(default)]
    pub resource: Option<String>,
    /// The relationship's edge handle (UUID from `assert_relationship`). Required when `target` is `edge`.
    #[serde(default)]
    pub edge_handle: Option<Uuid>,
    /// The facet row's id, as `facets_read` returned it. Required.
    pub property_id: Uuid,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// Retract one facet row owned by an edge.
///
/// Forwards to `DELETE /api/relationships/{edge_handle}/facets/{property_id}` through the
/// network door. The retraction is row-grain: one act per row, addressed by the id the facets
/// read carries. DELETE has no body, so the input's `ActInput` rides the request as query
/// params — passed verbatim, never defaulted.
pub async fn facet_retract(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: FacetRetractInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.target {
        FacetRetractTarget::Resource => Err(rmcp::ErrorData::invalid_params(
            "facet_retract applies to target=edge only: a resource's facet rows are \
             projector-minted surrogates whose ids are not stable identities, so they have no \
             retract-by-id. Use facet_set to overwrite a resource facet instead."
                .to_string(),
            None,
        )),
        FacetRetractTarget::Edge => {
            let edge_handle = input.edge_handle.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "target=edge requires `edge_handle`".to_string(),
                    None,
                )
            })?;
            let client = svc.relay_client(parts)?;
            let ack = client
                .facets()
                .retract_on_edge(edge_handle, input.property_id, &input.act)
                .await
                .across_auth(|e| map_err(e, "facet_retract"))?;

            Ok(CallToolResult::success(vec![
                rmcp::model::ContentBlock::text(to_text(&ack)),
            ]))
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_set_input_deserializes_without_act() {
        let json = serde_json::json!({
            "resource": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "values": {"summary": "example"},
            "weight": 0.5
        });
        let input: FacetSetInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.resource, "019e84ab-26ba-7560-9d34-c60d74a9fbe2");
        assert_eq!(
            serde_json::Value::Object(input.values),
            serde_json::json!({"summary": "example"})
        );
        assert_eq!(input.weight, Some(0.5));
        assert!(input.act.into_act_context().expect("assembles").is_empty());
    }

    #[test]
    fn facet_set_input_deserializes_with_act_authorship_fields() {
        let json = serde_json::json!({
            "resource": "foo-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "values": {"summary": "example"},
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "derived from ingest",
            "confidence": "confident",
        });
        let input: FacetSetInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.resource, "foo-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
        assert_eq!(input.weight, None);
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Confident)
        );
        assert!(input.act.invocation_id.is_some());
        let ctx = input.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }

    /// The retract input parses the unified shape: `target=edge` + `edge_handle` +
    /// `property_id`, with the act fields flattened.
    #[test]
    fn facet_retract_input_deserializes_the_edge_target() {
        let json = serde_json::json!({
            "target": "edge",
            "edge_handle": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "property_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
        });
        let input: FacetRetractInput = serde_json::from_value(json).unwrap();
        assert!(matches!(input.target, FacetRetractTarget::Edge));
        assert_eq!(
            input.edge_handle,
            Some("019e84ab-26ba-7560-9d34-c60d74a9fbe2".parse().unwrap())
        );
        assert_eq!(
            input.property_id,
            "019e84ab-26ba-7560-9d34-c60d74a9fbe3"
                .parse::<Uuid>()
                .unwrap()
        );
        assert!(input.act.invocation_id.is_some());
    }

    /// `target=resource` parses (the schema admits it) — the refusal is the handler's naming
    /// refusal, not a deserialization failure, so an agent caller gets the reason rather than
    /// a schema error.
    #[test]
    fn facet_retract_input_admits_target_resource_for_the_handler_to_refuse() {
        let json = serde_json::json!({
            "target": "resource",
            "resource": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "property_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
        });
        let input: FacetRetractInput = serde_json::from_value(json).unwrap();
        assert!(matches!(input.target, FacetRetractTarget::Resource));
    }
}
