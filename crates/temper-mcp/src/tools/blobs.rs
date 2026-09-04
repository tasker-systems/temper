//! Blob tools — read (read | list) and manage (commit | relate).
//!
//! Each action mirrors one HTTP endpoint from `temper-api/src/handlers/blobs.rs` and
//! dispatches through `blob_service` — the same service layer the HTTP handlers use, so the
//! gates (the NAMED predicates `blob_readable_by_profile` / `edges_visible_to` and the home
//! standing/peer gates) and every refusal vocabulary live there, never restated here.
//!
//! MCP carries JSON only, so bytes ride base64: `commit` takes the content base64-encoded
//! (the server's single-request threshold still gates it — the segmented path beyond it is
//! the CLI's), and `read` returns the bytes base64-encoded under a read ceiling that names
//! the streaming surfaces. The acts thread `Surface::Mcp`: a commit or relation made here is
//! attributed to the caller's `<handle>@mcp` emitter entity, per the relationship tools'
//! emitter-marker precedent.

use base64::Engine as _;
use futures::StreamExt as _;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::authorship::ActInput;
use temper_core::types::blob::{BlobRelationAssertRequest, BlobRelationDirection};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::{BlobId, ProfileId};
use temper_services::error::ApiError;
use temper_workflow::operations::Surface;

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────────────────

/// Which blob read action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum BlobReadAction {
    /// Read one blob's bytes back, whole — base64 in the result (the blob's stored media
    /// type and byte count ride alongside).
    Read,
    /// List the blobs you can read, optionally scoped to one home anchor. The response IS
    /// your readable set — never a discovery oracle.
    List,
}

/// MCP input for blob_read (actions: `read`, `list`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlobReadInput {
    /// Which blob read action to perform.
    pub action: BlobReadAction,
    /// The blob's UUID. Required for `read`; ignored for `list`.
    #[serde(default)]
    pub blob_id: Option<Uuid>,
    /// Optional home scope for `list`: `kb_contexts` or `kb_cogmaps`. With `home_id`, scopes
    /// the list to blobs homed in that anchor; absent, the list is every blob you can read.
    #[serde(default)]
    pub home_table: Option<String>,
    /// The home anchor's UUID (with `home_table` — the two are a pair). For `list` only.
    #[serde(default)]
    pub home_id: Option<Uuid>,
}

/// Which blob manage action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum BlobManageAction {
    /// Commit bytes as a blob (get-or-create on the content hash, PER-HOME: a re-commit of
    /// bytes this home already holds returns the same id — always a row you can read; the
    /// same bytes in someone else's scope are that principal's own row and never surface
    /// here).
    Commit,
    /// Assert one relation between a blob and another anchor. Retraction rides the
    /// incumbent fold endpoint (`edge fold`), not this tool.
    Relate,
}

/// MCP input for blob_manage (actions: `commit`, `relate`).
///
/// **Action → required fields:**
/// - `commit`: `home_table`, `home_id`, `content_type`, `content` (base64)
/// - `relate`: `blob_id`, `peer_table`, `peer_id`, `edge_kind`, `polarity`, `label`, `weight`
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlobManageInput {
    /// Which blob manage action to perform.
    pub action: BlobManageAction,
    /// `kb_contexts` or `kb_cogmaps` — the home the blob commits into (a blob needs a
    /// home, D2). Required for `commit`; ignored otherwise.
    #[serde(default)]
    pub home_table: Option<String>,
    /// The home anchor's UUID. Required for `commit`; ignored otherwise.
    #[serde(default)]
    pub home_id: Option<Uuid>,
    /// The media type the blob commits under — the server allowlist-checks it (D9) and
    /// refuses over its single-request threshold, naming the segmented path (the CLI's).
    /// Required for `commit`; ignored otherwise.
    #[serde(default)]
    pub content_type: Option<String>,
    /// The bytes to commit, base64-encoded (MCP carries JSON only). Required for `commit`;
    /// ignored otherwise.
    #[serde(default)]
    pub content: Option<String>,
    /// The blob's UUID. Required for `relate`; ignored otherwise.
    #[serde(default)]
    pub blob_id: Option<Uuid>,
    /// Which end of the edge the blob occupies — `blob_as_source` (the natural
    /// `figure_of`-shaped act) or `blob_as_target` (the derivation-source act, resource →
    /// blob). Defaults to `blob_as_source`, the wire's default. For `relate`.
    #[serde(default)]
    pub direction: Option<BlobRelationDirection>,
    /// The peer endpoint's table — `kb_resources`, `kb_cogmaps` or `kb_blobs`. Required for
    /// `relate`; ignored otherwise.
    #[serde(default)]
    pub peer_table: Option<String>,
    /// The peer endpoint's UUID. Required for `relate`; ignored otherwise.
    #[serde(default)]
    pub peer_id: Option<Uuid>,
    /// Structural edge kind — one of `express`, `contains`, `leads_to`, `near`. Required
    /// for `relate`; ignored otherwise.
    #[serde(default)]
    pub edge_kind: Option<EdgeKind>,
    /// Edge direction sign — `forward` or `inverse`. Required for `relate`; ignored
    /// otherwise.
    #[serde(default)]
    pub polarity: Option<Polarity>,
    /// Human-readable relation label (e.g. `figure_of`, `derivation_source`). Required for
    /// `relate`; ignored otherwise.
    #[serde(default)]
    pub label: Option<String>,
    /// Numeric edge weight (0.0–1.0 by convention). Required for `relate`; ignored
    /// otherwise.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship (`relate` only —
    /// a commit is a keyed get-or-create, not an authored act). Flattened top-level keys;
    /// all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

// ── Output struct ──────────────────────────────────────────────────────────────

/// The result of a blob `read`: the bytes base64-encoded under their stored media type.
/// `content_hash` is the bare sha256 hex — the proof that what is retrieved is what was
/// committed (`blob-bytes-retrievable-whole`).
#[derive(serde::Serialize)]
struct BlobReadResult {
    blob_id: Uuid,
    content_hash: String,
    content_type: String,
    content_bytes: i64,
    content_base64: String,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Map a blob-service error onto an MCP error, in the contexts-tool mapper's shape:
/// `NotFound` (the invisible-or-absent face the visibility gates deliberately render),
/// `Conflict` and `BadRequest` (the vocabularies — threshold, cap, allowlist, home scope,
/// `blob_relate:` peer table, finalize tokens) become invalid-params so the agent sees an
/// actionable message rather than an opaque internal error.
///
/// `Forbidden` and `NotFound` must stay **distinguishable** here: the home authority gate
/// answers 403 to a caller who can read the blob's home but not author it, and 404 to one
/// who cannot see it — collapsing the two would discard the disclosure distinction the
/// service draws.
fn map_api_error(action: &str, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("{action}: {msg}"), None)
        }
        ApiError::BadRequest(msg) | ApiError::Conflict(msg) => {
            rmcp::ErrorData::invalid_params(format!("{action}: {msg}"), None)
        }
        // The bare `Forbidden` the home authority gate returns (`check_home_authorable` —
        // `context_authorable_by_profile` / `cogmap_authorable_by_profile`): name what would
        // make the act acceptable, in the gate's own terms.
        ApiError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!(
                "{action}: the blob's home anchor is readable but not authorable — relate or \
                 commit from a home you can author (own it, or hold an authoring role in it)"
            ),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action} failed: {other}"), None),
    }
}

/// The scrub for a MID-STREAM read failure (N1, 2026-09-03 review): the stream error is
/// third-party content — every provider chunk error embeds the content-addressed pathname,
/// and reqwest's Display can attach the provider host (`for url (…)`) — and unlike the HTTP
/// face (headers already sent, the body-stream error aborts server-side), the MCP face
/// consumes the stream INSIDE the tool, so the raw text would otherwise render in the tool
/// result and breach D6's "the provider address never appears anywhere in the response".
///
/// The F9 posture, spelled for this face: the full error reaches `tracing`, the tool result
/// names only the DOOR. The error is NOT dropped — it stays a visible internal error, so a
/// truncated read can never masquerade as a short blob.
fn scrub_stream_error(action: &str, e: impl std::fmt::Display) -> rmcp::ErrorData {
    tracing::error!(
        context = action,
        error = %e,
        "blob read stream failed (scrubbed from the tool result)"
    );
    rmcp::ErrorData::internal_error(
        format!(
            "{action}: the blob's bytes could not be streamed from storage — the failure \
             detail is in the server log"
        ),
        None,
    )
}

/// The refusal for a CLEAN-BUT-SHORT read (C-S3, 2026-09-04 review): the stream ended
/// without an error yet delivered fewer bytes than the row declares. Same face as
/// [`scrub_stream_error`] — the counts and any provider signal are log-only, the tool
/// result names only the door — because the alternative is a success-shaped result that
/// renders the row's declared length beside a truncated `content_base64` (N7's
/// in-repo-checkable arm). A short read is a storage integrity failure, not caller
/// input, so it stays an internal error.
fn refuse_short_stream(action: &str, collected: usize, declared: i64) -> rmcp::ErrorData {
    tracing::error!(
        context = action,
        collected,
        declared,
        "blob read stream ended short of the declared length (scrubbed from the tool result)"
    );
    rmcp::ErrorData::internal_error(
        format!(
            "{action}: the blob's bytes came back short of their declared length from \
             storage — the failure detail is in the server log"
        ),
        None,
    )
}

/// The configured store + config pair every blob action needs, or the shared disabled
/// refusal (`blob_service::blob_disabled` — spelled once, every surface hears the same
/// voice). Absent, not broken: the `NullBroker` posture.
fn blob_parts(
    svc: &TemperMcpService,
    action: &str,
) -> Result<
    (
        std::sync::Arc<dyn temper_substrate::blob_store::BlobStore>,
        temper_services::config::BlobConfig,
    ),
    rmcp::ErrorData,
> {
    let store = svc
        .api_state
        .blob_store
        .clone()
        .ok_or_else(|| map_api_error(action, svc.api_state.blob_refusal()))?;
    let config = svc
        .api_state
        .config
        .blob
        .clone()
        .ok_or_else(|| map_api_error(action, svc.api_state.blob_refusal()))?;
    Ok((store, config))
}

// ── Read handlers ──────────────────────────────────────────────────────────────

pub async fn blob_read(
    svc: &TemperMcpService,
    input: BlobReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        BlobReadAction::Read => read_blob(svc, input).await,
        BlobReadAction::List => list_blobs(svc, input).await,
    }
}

async fn read_blob(
    svc: &TemperMcpService,
    input: BlobReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    const ACTION: &str = "blob_read";
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);
    let blob_id = input.blob_id.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("read requires `blob_id`".to_string(), None)
    })?;
    let (store, config) = blob_parts(svc, ACTION)?;

    let (blob, stream) = temper_services::services::blob_service::read_through(
        &svc.api_state.pool,
        store.as_ref(),
        caller,
        BlobId::from(blob_id),
    )
    .await
    .map_err(|e| map_api_error(ACTION, e))?;

    // The read ceiling is the single-request threshold already in force (D7) — one number,
    // declared where the operator set it. Beyond it, point at the surfaces that stream
    // rather than pulling megabytes through a JSON tool result.
    if blob.content_bytes > config.single_request_max_bytes as i64 {
        return Err(rmcp::ErrorData::invalid_params(
            format!(
                "{ACTION}: this blob is {} bytes against a {ACTION} ceiling of {} bytes — \
                 read it through the API (GET /api/blobs/{blob_id}) or the CLI \
                 (`temper blob get`), which stream",
                blob.content_bytes, config.single_request_max_bytes
            ),
            None,
        ));
    }

    let mut bytes: Vec<u8> = Vec::with_capacity(blob.content_bytes as usize);
    let mut stream = stream;
    while let Some(chunk) = stream
        .next()
        .await
        .transpose()
        .map_err(|e| scrub_stream_error(ACTION, e))?
    {
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != blob.content_bytes as usize {
        return Err(refuse_short_stream(ACTION, bytes.len(), blob.content_bytes));
    }

    let result = BlobReadResult {
        blob_id,
        content_hash: blob.content_hash,
        content_type: blob.content_type,
        content_bytes: blob.content_bytes,
        content_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&result),
    )]))
}

async fn list_blobs(
    svc: &TemperMcpService,
    input: BlobReadInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    const ACTION: &str = "blob_list";
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);
    if svc.api_state.blob_store.is_none() {
        return Err(map_api_error(ACTION, svc.api_state.blob_refusal()));
    }

    // The home-scope strings pass through verbatim — the pair constraint and the
    // two-kind vocabulary live in the service (the `parse_home` rule).
    let rows = temper_services::services::blob_service::list_blobs(
        &svc.api_state.pool,
        caller,
        input.home_table,
        input.home_id,
    )
    .await
    .map_err(|e| map_api_error(ACTION, e))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&rows),
    )]))
}

// ── Manage handlers ────────────────────────────────────────────────────────────

pub async fn blob_manage(
    svc: &TemperMcpService,
    input: BlobManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        BlobManageAction::Commit => commit_blob(svc, input).await,
        BlobManageAction::Relate => relate_blob(svc, input).await,
    }
}

async fn commit_blob(
    svc: &TemperMcpService,
    input: BlobManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    const ACTION: &str = "blob_commit";
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);
    let home_table = input.home_table.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("commit requires `home_table`".to_string(), None)
    })?;
    let home_id = input.home_id.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("commit requires `home_id`".to_string(), None)
    })?;
    let content_type = input.content_type.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("commit requires `content_type`".to_string(), None)
    })?;
    let content_b64 = input.content.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("commit requires `content` (base64)".to_string(), None)
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_b64.as_bytes())
        .map_err(|e| {
            rmcp::ErrorData::invalid_params(
                format!(
                    "{ACTION}: `content` is not valid base64 — the bytes ride base64 \
                         because MCP carries JSON only: {e}"
                ),
                None,
            )
        })?;
    let content_bytes = bytes.len() as i64;
    let (store, config) = blob_parts(svc, ACTION)?;

    // The home strings pass through verbatim — the parse (and its one-voice refusal) lives
    // in the service, shared with the HTTP handler. `Surface::Mcp` is the emitter marker:
    // the act is attributed to the caller's `<handle>@mcp` entity.
    let outcome = temper_services::services::blob_service::commit_blob(
        &svc.api_state.pool,
        store.as_ref(),
        &config,
        temper_services::services::blob_service::BlobCommitCommand {
            caller,
            home_table: Some(home_table),
            home_id: Some(home_id.to_string()),
            content_type,
            bytes: bytes.into(),
            surface: Surface::Mcp,
        },
    )
    .await
    .map_err(|e| map_api_error(ACTION, e))?;

    let response = temper_core::types::blob::BlobCommitResponse {
        blob_id: outcome.blob_id,
        content_hash: outcome.content_hash,
        // N2: the row's STORED media type — the first committer's on a dedup hit.
        content_type: outcome.content_type,
        content_bytes,
        deduped: outcome.deduped,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}

async fn relate_blob(
    svc: &TemperMcpService,
    input: BlobManageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    const ACTION: &str = "blob_relate";
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);
    let blob_id = input.blob_id.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `blob_id`".to_string(), None)
    })?;
    let peer_table = input.peer_table.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `peer_table`".to_string(), None)
    })?;
    let peer_id = input.peer_id.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `peer_id`".to_string(), None)
    })?;
    let edge_kind = input.edge_kind.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `edge_kind`".to_string(), None)
    })?;
    let polarity = input.polarity.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `polarity`".to_string(), None)
    })?;
    let label = input.label.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `label`".to_string(), None)
    })?;
    let weight = input.weight.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("relate requires `weight`".to_string(), None)
    })?;
    if svc.api_state.blob_store.is_none() {
        return Err(map_api_error(ACTION, svc.api_state.blob_refusal()));
    }

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let req = BlobRelationAssertRequest {
        // The wire's default — an omitted direction is the natural `figure_of`-shaped act.
        direction: input.direction.unwrap_or_default(),
        peer_table,
        peer_id,
        edge_kind,
        polarity,
        label,
        weight,
        act: ActInput::default(),
    };

    let ack = temper_services::services::blob_service::relate_blob(
        &svc.api_state.pool,
        caller,
        BlobId::from(blob_id),
        &req,
        act,
        Surface::Mcp,
    )
    .await
    .map_err(|e| map_api_error(ACTION, e))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate the tool input schema the same way rmcp does at runtime
    /// (`SchemaSettings::draft2020_12`, ref-based generator) — the
    /// `relationships.rs` harness. Scalar enums emitted as `$ref` into `$defs`
    /// reach the Anthropic tool-use layer with no type signal and come back as `null`.
    fn rmcp_schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap()
    }

    /// A schema field must inline its string enum rather than reference it via `$ref` —
    /// the Anthropic $ref constraint. Two inline shapes are admissible, and the incumbent
    /// tools advertise both: the flat `{"type":"string","enum":[…]}` form the core
    /// `schemars(inline)` enums produce, and the `oneOf`-of-string-consts form a locally
    /// `#[schemars(inline)]` enum produces (the action enums' advertised shape). What may
    /// NOT pass is a `$ref`: the client sees no values and sends `null`.
    fn assert_inline_string_enum(field: &serde_json::Value, variants: &[&str]) {
        assert!(
            field.get("$ref").is_none(),
            "field must be inlined, not a $ref: {field}"
        );
        // Flat shape: {"type": "string", "enum": [...]}. An Option-wrapped enum
        // (`#[serde(default)]` fields) adds `null` — `type` becomes a list and `null`
        // joins the variants; both still name every value inline.
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
        // oneOf shape: [{"const": "...", "type": "string"}, ...]
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

    #[test]
    fn blob_read_input_deserializes_both_actions() {
        let read: BlobReadInput = serde_json::from_value(serde_json::json!({
            "action": "read",
            "blob_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2"
        }))
        .unwrap();
        assert!(matches!(read.action, BlobReadAction::Read));
        assert!(read.blob_id.is_some());

        let list: BlobReadInput = serde_json::from_value(serde_json::json!({
            "action": "list",
            "home_table": "kb_contexts",
            "home_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe3"
        }))
        .unwrap();
        assert!(matches!(list.action, BlobReadAction::List));
        assert_eq!(list.home_table.as_deref(), Some("kb_contexts"));
    }

    #[test]
    fn blob_manage_input_deserializes_commit_and_relate() {
        let commit: BlobManageInput = serde_json::from_value(serde_json::json!({
            "action": "commit",
            "home_table": "kb_contexts",
            "home_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
            "content_type": "image/png",
            "content": "aGVsbG8="
        }))
        .unwrap();
        assert!(matches!(commit.action, BlobManageAction::Commit));
        assert_eq!(commit.content_type.as_deref(), Some("image/png"));

        let relate: BlobManageInput = serde_json::from_value(serde_json::json!({
            "action": "relate",
            "blob_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "peer_table": "kb_resources",
            "peer_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe4",
            "edge_kind": "express",
            "polarity": "forward",
            "label": "figure_of",
            "weight": 1.0,
            "confidence": "confident",
            "reasoning": "the figure renders this data"
        }))
        .unwrap();
        assert!(matches!(relate.action, BlobManageAction::Relate));
        // `direction` is absent in this JSON — the wire's `#[serde(default)]` posture: the
        // FIELD is `None` here, and the handler resolves the omission to the
        // `blob_as_source` default (the sibling test pins that resolution).
        assert_eq!(relate.direction, None);
        // The authorship fields must survive the flatten.
        let ctx = relate.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }

    #[test]
    fn blob_manage_relate_direction_defaults_to_the_wires_default() {
        let relate: BlobManageInput = serde_json::from_value(serde_json::json!({
            "action": "relate",
            "blob_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "peer_table": "kb_resources",
            "peer_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe4",
            "edge_kind": "express",
            "polarity": "forward",
            "label": "figure_of",
            "weight": 1.0
        }))
        .unwrap();
        assert_eq!(relate.direction, None);
        // The handler resolves the omission to the wire default.
        assert_eq!(
            relate.direction.unwrap_or_default(),
            BlobRelationDirection::BlobAsSource
        );
    }

    #[test]
    fn blob_read_schema_inlines_its_action_enum() {
        let schema = rmcp_schema_for::<BlobReadInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once enums are inlined: {schema}"
        );
        assert_inline_string_enum(&schema["properties"]["action"], &["read", "list"]);
    }

    #[test]
    fn blob_manage_schema_inlines_every_enum() {
        let schema = rmcp_schema_for::<BlobManageInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once enums are inlined: {schema}"
        );
        let props = &schema["properties"];
        assert_inline_string_enum(&props["action"], &["commit", "relate"]);
        assert_inline_string_enum(
            &props["edge_kind"],
            &["express", "contains", "leads_to", "near"],
        );
        assert_inline_string_enum(&props["polarity"], &["forward", "inverse"]);
        assert_inline_string_enum(&props["direction"], &["blob_as_source", "blob_as_target"]);
    }

    /// N1 (2026-09-03 review): a mid-stream provider failure is scrubbed to a
    /// door-named static message — the provider's own text (the content-addressed
    /// pathname, the host) must never reach the tool result — and the failure is NOT
    /// dropped: it stays an internal error, so a truncated read can never pass as a
    /// short blob. FAILS IF: the raw error is formatted into the rmcp error again, or
    /// the scrub swallows the error into a success.
    #[test]
    fn a_mid_stream_provider_failure_is_scrubbed_to_a_door_named_error() {
        const PROVIDER_TEXT: &str = "blob provider read: 503 Service Unavailable: \
             {\"code\":\"store_maintenance\",\"message\":\"SECRET-PROVIDER-TEXT\"} \
             for url (https://blob-store.example.com/ab/cdef0123)";
        let err = scrub_stream_error("blob_read", anyhow_error_for_test(PROVIDER_TEXT));

        let msg = err.message;
        assert!(
            !msg.contains("SECRET-PROVIDER-TEXT")
                && !msg.contains("store_maintenance")
                && !msg.contains("blob-store.example.com")
                && !msg.contains("cdef0123"),
            "the provider's own text must never reach the tool result; message was {msg}"
        );
        assert!(
            msg.contains("blob_read"),
            "the scrubbed message still names the DOOR so an operator can route it: {msg}"
        );
        assert_eq!(
            err.code,
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "a mid-stream failure stays a visible internal error — never a success"
        );
    }

    /// C-S3 (2026-09-04 review): a clean-but-short provider stream must never yield a
    /// declared length beside a truncated base64 body — the length mismatch refusals,
    /// scrubbed to the door, the counts log-only. FAILS IF: the mismatch renders as a
    /// success (the result carrying `content_bytes` and a short `content_base64`), or
    /// the scrub leaks more than the door's static sentence.
    #[test]
    fn a_clean_but_short_stream_refuses_instead_of_returning_the_declared_length() {
        let err = refuse_short_stream("blob_read", 3, 10);

        let msg = err.message;
        assert!(
            !msg.contains("3") && !msg.contains("10"),
            "the counts are log-only, the tool result names only the door: {msg}"
        );
        assert!(
            msg.contains("blob_read") && msg.contains("short of their declared length"),
            "the refusal names the door and the failure class: {msg}"
        );
        assert_eq!(
            err.code,
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "a short read stays a visible internal error — never a success"
        );
    }

    /// Stand-in for the anyhow error a provider chunk stream actually carries: the
    /// Display chain is what the scrub sees, and what N1 leaked.
    fn anyhow_error_for_test(text: &str) -> TestStreamError {
        TestStreamError(text.to_string())
    }

    struct TestStreamError(String);

    impl std::fmt::Display for TestStreamError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
}
