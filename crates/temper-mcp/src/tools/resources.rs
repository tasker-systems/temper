//! Resource tools — unified CRUD with name-based resolution and optional content.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G3a-prime, the network door):
//! every handler drives a per-request temper-client HTTP relay — built from the request's
//! `Parts` (the edge-verified bearer re-issued, the service credential and attribution
//! carrier set by the relay, design §D6) — and never touches the pool or a service
//! function directly. What stays MCP-local is input validation, `fields` projection, and
//! response shaping; the refusal-kind mapping below is carried over arm-for-arm from the
//! direct-service binding, with the post-edge authentication arms (deactivated /
//! machine-gate / expired-in-flight) split from the preserved 401 body by
//! `service::map_post_edge_auth`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_client::TemperClient;
use temper_core::context_ref::parse_context_ref;
// Still direct ONLY inside `build_create_command`, whose remaining caller is the ingest
// family's `ingest_begin` (not this beat's scope). The resources handlers below never
// touch it — their execution crosses the door.
use temper_core::types::authorship::ActInput;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::provenance::ProvenanceSource;
use temper_core::types::resource_view::{ResourceSection, ResourceView, SectionSet};
#[cfg(test)]
use temper_core::types::workflow_job::EmbeddingStatus;
use temper_services::services::context_service::resolve_context_ref;
use temper_workflow::operations::{BodyUpdate, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

use crate::service::{AcrossAuth, TemperMcpService};

/// Schemars `schema_with` for every `open_meta` input field.
///
/// `open_meta` is a free-form JSON object, held as `serde_json::Value` at
/// runtime. The default `JsonSchema` impl for `Value` advertises **no type** at
/// all, and some MCP clients, seeing no `type`, serialize the object as a JSON
/// **string** (`"{}"`) instead of an object — which the server then rejects with
/// `open_meta: "{}" is not of type "object"`. Advertising `type: object` fixes
/// the encoding at the client while `additionalProperties: true` keeps the tier
/// genuinely free-form (any key is allowed and stored).
///
/// We reuse the canonical recognized-conventions schema — the same
/// `open_meta.schema.json` that `describe_open_meta` serves — so the recognized
/// keys (`tags`, `keywords`, `descriptor`, `date`, …) are advertised as hints
/// from a single source of truth, with no risk of drift. `$schema`/`$id` are
/// stripped: they identify a standalone schema document, not an inlined
/// subschema, and would only confuse a client's schema resolution. A hardcoded
/// `type: object` fallback guarantees the load-bearing part of the fix even if
/// the canonical schema ever failed to parse.
fn open_meta_input_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    fn recognized_conventions() -> Option<schemars::Schema> {
        let mut value = temper_workflow::schema::open_meta_schema_value().ok()?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("$schema");
            obj.remove("$id");
        }
        schemars::Schema::try_from(value).ok()
    }

    recognized_conventions().unwrap_or_else(|| {
        schemars::json_schema!({
            "type": "object",
            "description": "Open (caller-defined) frontmatter as a JSON object. Free-form: any key is allowed and stored.",
            "additionalProperties": true,
        })
    })
}

// ── Input structs ──────────────────────────────────────────────────

/// MCP input for create_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    /// Context ref (UUID or `@owner/slug`), resolved server-side.
    /// Bare names (no `@` prefix, not a UUID) are rejected. Mutually exclusive
    /// with `cogmap`; supply exactly one home.
    #[serde(default)]
    pub context_ref: Option<String>,
    /// Cognitive-map ref (UUID or decorated `slug-<uuid>`) to home the resource
    /// in. Mutually exclusive with `context_ref`; supply exactly one home.
    #[serde(default)]
    pub cogmap: Option<String>,
    /// Human-readable doc type name (e.g. "task", "session", "research").
    pub doc_type_name: String,
    /// Resource title.
    pub title: String,
    /// Optional markdown content body. Processed through the ingest
    /// pipeline (chunk + embed) synchronously on create.
    pub content: Option<String>,
    /// Block-provenance sources this body was distilled from: resource refs (UUID or decorated) or
    /// external http/https URLs. Each becomes a provenance record on the created resource's body
    /// block (list position → accretion `seq`). Requires `content` — with no body block there is
    /// nothing to attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Optional goal link: a ref (UUID or decorated `slug-<uuid>`) of the goal this resource
    /// advances. Projects a live `advances`→goal edge on create. Relationship-fated, not
    /// metadata — first-class here, never a `managed_meta` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Optional origin URI. Defaults to `mcp://agent/{uuid}`. An external (http/https) origin URI
    /// with `content` but no explicit `sources` seeds a Remote block-provenance record pointing at
    /// it (issue #352), so a resource distilled from a URL is citation-grade by default.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
    /// Managed workflow/provenance frontmatter — a **closed, temper-owned
    /// vocabulary** of optional `temper-*` keys: stage/mode/effort/status/seq/
    /// branch/pr/llm-model/llm-run/provenance. Identity (`title`), type
    /// (`doc_type_name`), and home (`context_ref`/`cogmap`) are first-class
    /// fields on this input, not metadata. An unknown key is rejected;
    /// caller-defined ("bring-your-own") fields belong in `open_meta`.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (caller-defined) frontmatter as a free-form JSON **object**. Any key
    /// is allowed and stored; recognized keys (`tags`, `keywords`, `descriptor`,
    /// `date`, …) are advertised for shape/ranking but not required. A `null`
    /// value is refused on create (null deletes a key on update — omit the key here).
    #[serde(default)]
    #[schemars(schema_with = "open_meta_input_schema")]
    pub open_meta: Option<serde_json::Value>,
    /// Owner-scoped create idempotency key (issue #581). A client-minted opaque token (a UUIDv7 by
    /// convention): a retried create carrying the same key converges on the already-committed
    /// resource instead of minting a duplicate, deduped on `(owner, key)`. `None` = an ordinary,
    /// non-idempotent create. The server mints the resource id; the key never supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<uuid::Uuid>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship
    /// (`reasoning`/`confidence`/`rationale`/`persona`/`model`). Flattened as top-level keys;
    /// all optional. `confidence` is required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for get_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub id: String,
    /// If true, includes the full reconstituted markdown content.
    pub include_content: Option<bool>,
    /// Subselect top-level response keys. Anchor key `id` is always
    /// preserved. Nested paths (containing `.`) rejected with a hint
    /// pointing at `jq` — MCP callers should perform deeper projection
    /// at their own end. When None or empty, no filtering is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

/// MCP input for get_block_provenance.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBlockProvenanceInput {
    /// The resource whose per-block provenance to read (UUID).
    pub resource: Uuid,
}

/// MCP input for get_block — the three-state block read.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBlockInput {
    /// The resource the block belongs to (UUID). A named successor of a folded block
    /// carries its own home as `home_resource_id` — pass THAT resource to address it.
    pub resource: Uuid,
    /// The content block to read (a block UUID). May address a folded block or a
    /// named successor.
    pub block_id: Uuid,
}

/// MCP input for resource_lineage (Ledger L2).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceLineageInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub id: String,
    /// Max hop distance to walk from the seed (default 16, clamped to 1..=64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i32>,
}

/// MCP input for annotate_resource (issue #355) — attach provenance sources without a body revise.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnotateResourceInput {
    /// The resource to annotate (UUID).
    pub id: Uuid,
    /// Sources to attach to the addressed block: resource refs (UUID or decorated) or external
    /// http/https URLs. A URL may carry a span-locator fragment (e.g. `…/doc.md#L120-L180`), which is
    /// preserved verbatim and surfaced by `get_block_provenance`. Position → accretion `seq`. Required
    /// and non-empty — an annotate with nothing to attribute is an error.
    pub sources: Vec<String>,
    /// Which content block to annotate (a block UUID). Omit to address the resource's sole non-folded
    /// body block (the default); required for a resource with more than one block. The block must
    /// belong to the resource and be non-folded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_block: Option<Uuid>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level keys;
    /// all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for list_resources.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context ref (UUID or @owner/slug). Bare context names are rejected.
    pub context_ref: Option<String>,
    /// Filter by doc type name (e.g. "task", "research").
    pub doc_type_name: Option<String>,
    /// Filter tasks by workflow stage: `backlog`, `in-progress`, `done`, `cancelled`.
    /// Meaningful only for `doc_type_name = "task"` — no other doc type carries a stage,
    /// so combining them matches nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Filter goals by lifecycle status: `active`, `completed`, `paused`, `cancelled`.
    /// Meaningful only for `doc_type_name = "goal"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Filter by tag. Returns resources carrying **every** tag listed (AND — each added tag
    /// narrows). Matching is exact per tag and case-insensitive. Unlike `stage` and `status`,
    /// tags are NOT doc-type-scoped — 14 doc types carry them — so this composes with any
    /// `doc_type_name`, or with none, to enumerate a whole axis in one call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Filter by goal: a ref (UUID or decorated `slug-<uuid>`) of a goal resource. Returns only
    /// resources linked to it via a live `advances`→goal edge (task-only in practice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Scope to resources homed in one or more cognitive maps (UUID or decorated refs). The result
    /// is the union across the maps. Mutually exclusive with `context_ref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap: Option<Vec<String>>,
    /// Max results (default 50, max 200).
    pub limit: Option<i64>,
    /// Pagination offset.
    pub offset: Option<i64>,
    /// Subselect top-level response keys for each row. Anchor key `id`
    /// is always preserved per row. Nested paths (containing `.`) are
    /// rejected with a hint pointing at `jq` — MCP callers should
    /// perform deeper projection at their own end. When None or empty,
    /// no filtering is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

/// MCP input for update_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New title.
    pub title: Option<String>,
    /// Set (or replace) the resource's goal: a ref (UUID or decorated `slug-<uuid>`) of the goal
    /// this resource advances. Folds any existing `advances`→goal edge and asserts the new one.
    /// Mutually exclusive with `clear_goal`. Omit to leave the goal edge untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Clear the resource's goal: when `true`, folds the current `advances`→goal edge, leaving it
    /// goal-less. The tri-state complement to `goal` (absent = untouched, `goal` = set/replace,
    /// `clear_goal` = retract). Mutually exclusive with `goal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_goal: Option<bool>,
    /// New markdown content. Replaces existing content and triggers
    /// re-processing.
    pub content: Option<String>,
    /// Block-provenance sources this body was distilled from: resource refs (UUID or decorated) or
    /// external http/https URLs. Each becomes a provenance record on the resource's body block (list
    /// position → accretion `seq`). Requires `content` — with no body update there is nothing to
    /// attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Which content block the body revise + `sources` target (a block UUID). Omit to address the
    /// resource's sole body block (the default); required to revise a resource that has more than one
    /// block. The block must belong to the resource and be non-folded. Requires `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_block: Option<Uuid>,
    /// Managed workflow/provenance frontmatter — a **closed, temper-owned
    /// vocabulary** of optional `temper-*` keys: stage/mode/effort/status/seq/
    /// branch/pr/llm-model/llm-run/provenance. Identity (`title`), type
    /// (`doc_type_name`), and home (`context_ref`/`cogmap`) are first-class
    /// fields on this input, not metadata. An unknown key is rejected;
    /// caller-defined ("bring-your-own") fields belong in `open_meta`.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (caller-defined) frontmatter as a free-form JSON **object**. Any key
    /// is allowed and stored; recognized keys (`tags`, `keywords`, `descriptor`,
    /// `date`, …) are advertised for shape/ranking but not required. An explicit
    /// **`null` value deletes the key** (in-band verb; unnamed keys are never
    /// touched — there is no whole-object replace).
    #[serde(default)]
    #[schemars(schema_with = "open_meta_input_schema")]
    pub open_meta: Option<serde_json::Value>,
    /// Additive open_meta patch: a JSON object whose values are **arrays**, each unioned
    /// with the list already stored rather than replacing it. Repeating an add is a no-op,
    /// so this expresses "make sure these are present".
    ///
    /// Use this to add a tag; use `open_meta` to replace or clear a list (`{"tags":[]}`).
    /// Sending `{"tags":["x"]}` in `open_meta` replaces every existing tag — correct when
    /// you mean to state the list in full, silent data loss when you meant to add one.
    #[serde(default)]
    pub open_meta_add: Option<serde_json::Value>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for update_resource_meta.
///
/// Use when the caller wants to change only a resource's frontmatter
/// (managed_meta / open_meta) without re-chunking or re-embedding the
/// body. This is the MCP peer of `PUT /api/resources/{id}/meta`.
///
/// `managed_meta` is a **closed, temper-owned vocabulary** — exactly the
/// optional `temper-*` Property keys (stage/mode/effort/status/seq/branch/pr/
/// llm-model/llm-run/provenance). Unknown keys are rejected; this path is
/// Property-only — identity (`title`/`slug`), type, and home are NOT accepted
/// here (change them via `update_resource`). `open_meta` stays a free-form JSON
/// object by design — the open tier accepts any key, and is advertised to
/// clients as `type: object` with `additionalProperties: true` (via the
/// `open_meta_input_schema` helper).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceMetaInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New managed (temper-*) frontmatter — a **closed, temper-owned
    /// vocabulary**. Only the typed temper-* keys are accepted; an unknown key
    /// is rejected. Caller-defined fields belong in `open_meta`.
    pub managed_meta: ManagedMeta,
    /// New open (caller-defined) frontmatter as a free-form JSON **object**. Any
    /// key is allowed and stored; recognized keys (`tags`, `keywords`,
    /// `descriptor`, `date`, …) are advertised for shape/ranking but not required.
    #[schemars(schema_with = "open_meta_input_schema")]
    pub open_meta: serde_json::Value,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for delete_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceInput {
    /// UUID of the resource to delete.
    pub id: Uuid,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

// ── Response types ─────────────────────────────────────────────────

/// Status of a create_resource operation.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateStatus {
    Created,
    Existing,
}

/// Typed response for create_resource.
#[derive(Debug, serde::Serialize)]
pub struct CreateResourceResponse {
    pub resource: ResourceView,
    pub status: CreateStatus,
}

// ── Response shaping ───────────────────────────────────────────────

/// Typed response for delete_resource.
#[derive(Debug, serde::Serialize)]
pub struct DeleteResourceResponse {
    pub deleted: bool,
    pub id: Uuid,
}

/// Typed response for update_resource_meta.
#[derive(Debug, serde::Serialize)]
pub struct UpdateResourceMetaResponse {
    pub updated: bool,
    pub id: Uuid,
}

/// MCP's `list_resources` envelope — the paging state a caller needs to know whether it is looking
/// at the whole set or a page of it.
///
/// **The three quantities the shipped agent skill instructs every agent to read — `total`,
/// `returned`, `truncated` — were emitted by the CLI alone.** MCP answered with a bare JSON array,
/// so an agent on this surface could not tell a complete answer from a truncated one and had no
/// signal that it must not conclude a resource is absent. Same defect class as the `ref` this task
/// also closes.
///
/// Every field but `rows` is carried through verbatim from
/// [`temper_workflow::types::resource::ResourceListResponse`] — `returned` and `truncated` are
/// derived once, in that type's `new`, so this surface cannot disagree with the HTTP one about what
/// `truncated` means (`offset + returned < total`, which is `false` on the last page of a walk).
///
/// `rows` is `Vec<serde_json::Value>` because the optional `fields` projection is dynamic by
/// construction — it returns whichever top-level keys the caller named. With no `fields` the values
/// are whole [`ResourceView`]s with the `open-meta` and `embedding-status` sections filled
/// (B1: the view itself carries the field, so the old flattened `EnrichedResource` envelope
/// dissolved into it — the wire keys are the same, at the same depth).
#[derive(Debug, serde::Serialize)]
pub struct ListResourcesResponse {
    pub rows: Vec<serde_json::Value>,
    /// The FILTERED match count — every row the filters admit, before `limit`/`offset`.
    pub total: i64,
    /// This page's row count.
    pub returned: i64,
    /// Are there matching rows beyond this page?
    pub truncated: bool,
    /// The effective page size applied, or `None` for an uncapped page.
    pub limit: Option<i64>,
    /// The offset this page starts at.
    pub offset: i64,
    /// Doc-type histogram over the filtered set.
    pub facets: temper_workflow::types::resource::ResourceFacets,
}

/// The sections every MCP single-resource read asks the door for.
///
/// `open-meta` is the incumbent ask (the view's open tier rode every response before the
/// one-seam migration); `embedding-status` is B1's — the view itself carries the field now, so
/// the response needs no second read and no wrapper shape. One constant, so `get`/`create`/
/// `update` cannot drift on which sections a response carries.
fn enriched_sections() -> SectionSet {
    [ResourceSection::OpenMeta, ResourceSection::EmbeddingStatus]
        .into_iter()
        .collect()
}

/// Read one resource back as the MCP surface answers it — the view with the sections the
/// tool asked for, through the door.
///
/// The single-resource read for `create`/`update`/`get`, so those three cannot drift on
/// which sections a response carries: the door's `GET /api/resources/{id}?sections=` fills
/// both tiers plus the derived embedding readiness on the row the gate already admitted,
/// and the body — a SECTION in the direct-service binding — arrives from `/content` only
/// when asked, as its own markdown part.
async fn enriched_view(
    client: &TemperClient,
    id: Uuid,
    include_content: bool,
) -> Result<(ResourceView, Option<String>), rmcp::ErrorData> {
    let view = client
        .resources()
        .get(id, Some(&enriched_sections()))
        .await
        .across_auth(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
        })?;
    let body_markdown = if include_content {
        Some(
            client
                .resources()
                .content(id)
                .await
                .across_auth(|e| {
                    rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
                })?
                .markdown,
        )
    } else {
        None
    };
    Ok((view, body_markdown))
}

// ── Helpers ────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Build the command's `BodyUpdate` from optional content + optional resource-id sources,
/// mapping each source id to `ProvenanceSource::Resource` (list position → accretion seq
/// downstream). Shared by create and update. Guards the parse-don't-validate invariant:
/// sources without a body block have nothing to attribute, so that combination is an
/// `invalid_params` error rather than a silent drop.
///
/// Direct-binding remnant: its only remaining caller is `build_create_command` (the
/// ingest family). The migrated resources handlers use [`resolve_sources`] instead —
/// the door carries content and sources as separate flat fields.
fn provenance_body(
    content: Option<String>,
    sources: Option<Vec<String>>,
    content_block: Option<Uuid>,
) -> Result<Option<BodyUpdate>, rmcp::ErrorData> {
    match content {
        Some(content) if !content.is_empty() => {
            let mut body = BodyUpdate::new(content);
            // Classify each source (http/https URL → Remote, else ref → Resource) with the same shared
            // resolver the CLI uses; an unparseable value is a hard error, never a silent drop.
            body.sources = sources
                .unwrap_or_default()
                .iter()
                .map(|s| temper_workflow::operations::resolve_provenance_source(s))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("invalid sources value: {e}"), None)
                })?;
            body.content_block = content_block;
            Ok(Some(body))
        }
        _ => {
            if sources.is_some_and(|s| !s.is_empty()) {
                return Err(rmcp::ErrorData::invalid_params(
                    "sources supplied without content — there is no body block to attribute"
                        .to_owned(),
                    None,
                ));
            }
            if content_block.is_some() {
                return Err(rmcp::ErrorData::invalid_params(
                    "content_block supplied without content — there is no body revise to address"
                        .to_owned(),
                    None,
                ));
            }
            Ok(None)
        }
    }
}

/// Classify each wire source (http/https URL → Remote, else ref → Resource) with the
/// shared resolver the CLI uses; an unparseable value is a hard error, never a silent
/// drop. The migrated handlers' version of `provenance_body`'s classification loop.
fn resolve_sources(sources: Option<Vec<String>>) -> Result<Vec<ProvenanceSource>, rmcp::ErrorData> {
    sources
        .unwrap_or_default()
        .iter()
        .map(|s| temper_workflow::operations::resolve_provenance_source(s))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid sources value: {e}"), None))
}

/// The body-write pieces the PATCH-shaped update needs, in the door's flat form.
struct BodyParts {
    content: Option<String>,
    sources: Vec<ProvenanceSource>,
    content_block: Option<Uuid>,
}

/// The body-write guard, flat-field form for the door's PATCH shape: content and the
/// provenance fields travel separately on `ResourceUpdateRequest`, so the
/// sources/content_block-without-content refusals fire here, at the tool, exactly as
/// they did in the direct binding.
fn provenance_parts(
    content: Option<String>,
    sources: Option<Vec<String>>,
    content_block: Option<Uuid>,
) -> Result<BodyParts, rmcp::ErrorData> {
    match content {
        Some(content) if !content.is_empty() => Ok(BodyParts {
            content: Some(content),
            sources: resolve_sources(sources)?,
            content_block,
        }),
        _ => {
            if sources.as_ref().is_some_and(|s| !s.is_empty()) {
                return Err(rmcp::ErrorData::invalid_params(
                    "sources supplied without content — there is no body block to attribute"
                        .to_owned(),
                    None,
                ));
            }
            if content_block.is_some() {
                return Err(rmcp::ErrorData::invalid_params(
                    "content_block supplied without content — there is no body revise to address"
                        .to_owned(),
                    None,
                ));
            }
            Ok(BodyParts {
                content: None,
                sources: Vec::new(),
                content_block: None,
            })
        }
    }
}

// ── Tool handlers ──────────────────────────────────────────────────

/// Build the shared `CreateResource` command from an MCP create input: validate the owner format,
/// resolve the home anchor (running the cogmap producer gate before any write), derive the slug from
/// the title, default `origin_uri`, assemble the act context, and resolve the optional goal ref.
///
/// Sole caller since beat G3a: `tools::ingest::ingest_begin` (the ingest family) — a
/// segmented begin creates a resource by exactly the same rules `create_resource` used
/// to. `create_resource` itself now crosses the ingest door and builds its payload
/// separately; when the ingest family migrates, this helper and its duplicate shaping
/// die together.
pub(crate) async fn build_create_command(
    svc: &TemperMcpService,
    profile_id: ProfileId,
    input: CreateResourceInput,
) -> Result<CreateResource, rmcp::ErrorData> {
    let pool = &svc.api_state.pool;

    // Validate owner format if provided (stub for R11)
    if let Some(ref owner) = input.owner {
        if !owner.starts_with('@') && !owner.starts_with('+') {
            return Err(rmcp::ErrorData::invalid_params(
                "owner must start with @ (profile) or + (team)".to_string(),
                None,
            ));
        }
    }

    // Resolve the home anchor — exactly one of a cognitive map or a context.
    // Symmetric with the HTTP ingest handler: the cogmap branch runs the
    // producer write gate (auth before writes) before homing in the map.
    let home = match (input.cogmap.as_deref(), input.context_ref.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(rmcp::ErrorData::invalid_params(
                "context_ref and cogmap are mutually exclusive; supply exactly one home"
                    .to_string(),
                None,
            ));
        }
        (None, None) => {
            return Err(rmcp::ErrorData::invalid_params(
                "no home specified — supply exactly one of context_ref or cogmap".to_string(),
                None,
            ));
        }
        (Some(cogmap_ref), None) => {
            // Trailing-UUID-only resolution (no server lookup).
            let map = temper_workflow::operations::parse_ref(cogmap_ref)
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("invalid cogmap ref: {e}"), None)
                })?
                .0;
            // Auth before writes is unchanged and still runs — it lives one layer down, in
            // `DbBackend::create_resource`'s F1 gate (`check_cogmap_authorable`), which is the
            // ENFORCING copy on the shared write path and denies before any row is written.
            //
            // What used to be here was a second, redundant `cogmap_service::authorable_by_profile`
            // pre-check whose only job was to fail fast — and which, because it held a bare `bool`,
            // could only render *"not authorized to author in this cognitive map"*. That string
            // shadowed the gate: the create path is the most-used way into a map, and it answered in
            // the opaque voice this change exists to retire, no matter what the gate below decided.
            // A pre-check cannot be taught the disclosure rule without re-deriving the read probe
            // beside it — two copies of one policy, which is the drift this file's own conventions
            // forbid. So the fast-fail goes and the gate speaks.
            HomeAnchor::Cogmap(temper_core::types::ids::CogmapId::from(map))
        }
        (None, Some(context_ref)) => {
            // Parse + resolve the context ref (UUID or @owner/slug). Bare names are rejected.
            let cref = parse_context_ref(context_ref).map_err(|e| {
                rmcp::ErrorData::invalid_params(format!("invalid context_ref: {e}"), None)
            })?;
            let context = resolve_context_ref(pool, profile_id, &cref)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("context not found: {e}"), None)
                })?;
            HomeAnchor::Context(context)
        }
    };

    // Slug is §7-dissolved (never stored; addressing is trailing-UUID-only), so it is NOT a
    // caller input — always derived from the title via the one canonical slugifier, whose
    // output is validate_slug-conformant (ASCII, runs collapsed). (issue #307 Bug 2)
    let slug = temper_workflow::operations::sluggify(&input.title);

    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    let content = input.content.unwrap_or_default();

    // Identity travels first-class on the cmd (title/slug below); managed_meta is
    // Property-only. The caller-supplied managed_meta passes through untouched —
    // the DbBackend validation pipeline injects identity into the validation
    // document from the typed title/slug.
    let managed_meta = input.managed_meta.unwrap_or_default();

    // Create always writes a single new body block; per-block addressing is an update-only concern.
    let body = provenance_body(Some(content), input.sources, None)?;

    // Assemble the per-act correlation + authorship from the flattened discrete fields. The
    // shared assembler enforces "confidence required iff authorship supplied"; map its
    // BadRequest to invalid_params.
    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    // Resolve the optional goal ref client-side (trailing-UUID-only, like `edge assert`); the
    // backend projects the live `advances`→goal edge after create.
    let goal = input
        .goal
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = CreateResource {
        idempotency_key: input.idempotency_key,
        slug,
        doctype: input.doc_type_name,
        home,
        title: input.title,
        body,
        managed_meta,
        open_meta: input.open_meta,
        goal,
        origin_uri: Some(origin_uri),
        chunks_packed: None,
        content_hash: None,
        act,
        origin: Surface::Mcp,
    };

    Ok(cmd)
}

pub async fn create_resource(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: CreateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    // Validate owner format if provided (stub for R11) — input shaping stays MCP-local.
    if let Some(ref owner) = input.owner {
        if !owner.starts_with('@') && !owner.starts_with('+') {
            return Err(rmcp::ErrorData::invalid_params(
                "owner must start with @ (profile) or + (team)".to_string(),
                None,
            ));
        }
    }

    // Exactly one home, both expressed on the payload; precedence (cogmap beats
    // context_ref) and bare-name rejection are the ingest handler's, word for word.
    let home_cogmap_id = match input.cogmap.as_deref() {
        Some(cogmap_ref) => {
            if input.context_ref.is_some() {
                return Err(rmcp::ErrorData::invalid_params(
                    "context_ref and cogmap are mutually exclusive; supply exactly one home"
                        .to_string(),
                    None,
                ));
            }
            Some(
                temper_workflow::operations::parse_ref(cogmap_ref)
                    .map_err(|e| {
                        rmcp::ErrorData::invalid_params(format!("invalid cogmap ref: {e}"), None)
                    })?
                    .0,
            )
        }
        None => {
            if input.context_ref.is_none() {
                return Err(rmcp::ErrorData::invalid_params(
                    "no home specified — supply exactly one of context_ref or cogmap".to_string(),
                    None,
                ));
            }
            None
        }
    };

    // Slug is §7-dissolved — the ingest door derives it from the title server-side.
    // The `mcp://agent/{uuid}` origin default stays MCP-local (request shaping).
    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    let content = input.content.unwrap_or_default();
    let sources = resolve_sources(input.sources)?;
    // The sources-without-body guard, carried over from the direct binding — and it
    // must fire HERE: the ingest door carries sources only inside `BodyUpdate`, so an
    // empty body would silently DROP them instead of refusing.
    if content.is_empty() && !sources.is_empty() {
        return Err(rmcp::ErrorData::invalid_params(
            "sources supplied without content — there is no body block to attribute".to_owned(),
            None,
        ));
    }

    // The caller-supplied managed_meta is a typed input; the wire payload carries the
    // JSON value the ingest door re-parses server-side (typed at both ends).
    let managed_meta = match input.managed_meta {
        Some(m) => Some(serde_json::to_value(m).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("managed_meta failed to serialize: {e}"), None)
        })?),
        None => None,
    };

    let payload = temper_core::types::ingest::IngestPayload {
        title: input.title,
        origin_uri,
        context_ref: input.context_ref.unwrap_or_default(),
        home_cogmap_id,
        doc_type_name: input.doc_type_name,
        goal: input
            .goal
            .as_deref()
            .map(|r| {
                temper_workflow::operations::parse_ref(r)
                    .map(|parsed| parsed.0)
                    .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))
            })
            .transpose()?,
        content_hash: None,
        idempotency_key: input.idempotency_key,
        content,
        metadata: None,
        managed_meta,
        open_meta: input.open_meta,
        chunks_packed: None,
        sources,
        act: input.act,
        segmented: None,
    };

    let view = client.ingest().create(&payload).await.across_auth(|e| {
        match e {
        // F-2: placing a resource into a context requires WRITE on that context, and the
        // door's own gate enforces it for both home kinds. The sentences are the gate's,
        // carried arm-for-arm from the direct binding's error mapping.
        ClientError::ForbiddenDetail { message } => {
            rmcp::ErrorData::invalid_params(message, None)
        }
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            "Not authorized to create in this context: placing a resource requires write access, \
             and read access alone (watcher role, a read-only grant, a shared context, or \
             membership in an enclosing team) is not enough."
                .to_string(),
            None,
        ),
        // The service's own sentence, un-prefixed: the direct binding's tool wrapped the
        // resolver's failure with "context not found: ", a prefix the door does not
        // re-apply — the kind (invalid_params) and the gate are identical.
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(message, None),
        other => rmcp::ErrorData::internal_error(
            format!("Failed to create resource: {other}"),
            None,
        ),
        }
    })?;

    // Read back through the same door `get_resource` uses so the response carries both
    // tiers plus derived embedding status, exactly as the shape it replaced did.
    let (enriched, _) = enriched_view(&client, Uuid::from(view.id), false).await?;
    let response = CreateResourceResponse {
        resource: enriched,
        status: CreateStatus::Created,
    };
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&response)),
    ]))
}

/// Map a `ProjectionError` to an `rmcp::ErrorData` invalid-params response.
///
/// Centralises the error-boundary translation so both `get_resource` and
/// `list_resources` can call `.map_err(map_projection_err)?` without
/// duplicating the match arms.
fn map_projection_err(e: temper_core::projection::ProjectionError) -> rmcp::ErrorData {
    use temper_core::projection::ProjectionError;
    match e {
        ProjectionError::DottedPath { hint } => rmcp::ErrorData::invalid_params(
            format!("fields supports top-level keys only; use jq for nested projection: {hint}"),
            None,
        ),
        ProjectionError::EmptyField => {
            rmcp::ErrorData::invalid_params("fields contained an empty entry".to_string(), None)
        }
    }
}

// `get_resource` reads through the door's `GET /api/resources/{id}` — identity, the
// managed tier and the open tier come from one router answer, which is what makes this
// door and the HTTP one the same answer. `include_content` fetches the body from the
// door's `/content` read; it leaves the JSON and becomes its own markdown content part —
// an MCP client renders that, and inlining prose into the metadata object would make
// both harder to read.
pub async fn get_resource(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    let id = temper_workflow::operations::parse_ref(&input.id)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?
        .0;

    let include_content = input.include_content.unwrap_or(false);
    let (mut enriched, body_markdown) = enriched_view(&client, id, include_content).await?;
    // `take` rather than a clone: `content: None` omits the key, which is the shape the
    // caller expects when it did not ask for a body.
    let body_markdown = body_markdown.or_else(|| enriched.content.take());

    let enriched_value = serde_json::to_value(&enriched)
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None))?;

    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(enriched_value, fields, "id")
            .map_err(map_projection_err)?
    } else {
        enriched_value
    };

    let mut parts = vec![rmcp::model::ContentBlock::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "{}".to_string()),
    )];
    if let Some(markdown) = body_markdown {
        parts.push(rmcp::model::ContentBlock::text(markdown));
    }
    Ok(CallToolResult::success(parts))
}

/// Itemized per-block provenance for a resource, through the door. The access gate lives
/// in the SQL function behind the route — an unreadable resource yields an empty list.
pub async fn get_block_provenance(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: GetBlockProvenanceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    let rows = client
        .resources()
        .provenance(input.resource)
        .await
        .across_auth(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to read provenance: {e}"), None)
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&rows)),
    ]))
}

/// Read one content block by address — the three-state resolution (D-D1), through the
/// door. The two denial shapes stay DIFFERENT here, exactly as the tool description
/// promises: an absent address arrives as `Ok(BlockRead::Absent)` — the client
/// synthesizes it from the route's own block-naming 404, never from a foreign one — and
/// renders as data (`state: "absent"`), while a not-visible home arrives as
/// `ClientError::NotFound` and maps to `invalid_params` — denying existence, never 403.
/// A folded successor is addressed by calling this again with its own block id.
pub async fn get_block(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: GetBlockInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    let read = client
        .resources()
        .read_block(input.resource, input.block_id)
        .await
        .across_auth(|e| match e {
            ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
            other => rmcp::ErrorData::internal_error(format!("block read failed: {other}"), None),
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&read)),
    ]))
}

/// Ledger L2 — a resource's bidirectional `derived_from` lineage: what it derives
/// from (ancestors) and what derives from it (descendants), access-gated, through the
/// door. An unreadable/absent seed is a not-found error (not an empty leak).
pub async fn resource_lineage(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ResourceLineageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    let id = temper_workflow::operations::parse_ref(&input.id)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?
        .0;
    let depth = input.depth.unwrap_or(16).clamp(1, 64);

    let lineage = client
        .resources()
        .lineage(id, Some(depth))
        .await
        .across_auth(|e| match e {
            ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
            other => rmcp::ErrorData::internal_error(format!("lineage read failed: {other}"), None),
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&lineage)),
    ]))
}

pub async fn list_resources(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    // Resolve the optional goal filter ref client-side (trailing-UUID-only, like the write path).
    let goal = input
        .goal
        .as_deref()
        .map(|r| {
            temper_workflow::operations::parse_ref(r)
                .map(|parsed| parsed.0)
                .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))
        })
        .transpose()?;

    // Resolve the optional cogmap scope (each ref → UUID, trailing-UUID-only) into the CSV the list
    // params carry. Mutually exclusive with context_ref (a resource has one home).
    let cogmap_ids: Option<String> = match input.cogmap.as_deref() {
        Some(refs) if !refs.is_empty() => {
            if input.context_ref.is_some() {
                return Err(rmcp::ErrorData::invalid_params(
                    "context_ref and cogmap are mutually exclusive".to_string(),
                    None,
                ));
            }
            let mut ids = Vec::with_capacity(refs.len());
            for r in refs {
                let id = temper_workflow::operations::parse_ref(r).map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("bad cogmap ref {r:?}: {e}"), None)
                })?;
                ids.push(id.0.to_string());
            }
            Some(ids.join(","))
        }
        _ => None,
    };

    // Join the tag list into the CSV the list params carry (same transport constraint as
    // `cogmap_ids`). A tag containing a comma is refused rather than split: over this wire it
    // would silently become two tags and return a narrower set than asked for. Case folding and
    // trimming are the server's job (`filtered_visible_page`), so this door cannot disagree with
    // the CLI about what matches.
    let tags: Option<String> = match input.tags.as_deref() {
        Some(list) if !list.is_empty() => {
            for t in list {
                if t.contains(',') {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!("bad tag {t:?}: a tag may not contain a comma"),
                        None,
                    ));
                }
            }
            Some(list.join(","))
        }
        _ => None,
    };

    // Build list params — context_ref is resolved server-side by filtered_visible_page;
    // bare context names are rejected there (spec Decision 1).
    let params =
        temper_workflow::types::resource::ResourceListParams {
            context_ref: input.context_ref.clone(),
            doc_type_name: input.doc_type_name.clone(),
            stage: input.stage.clone(),
            status: input.status.clone(),
            tags,
            goal,
            cogmap_ids,
            limit: input.limit.or(Some(50)).map(|l| l.min(200)),
            offset: input.offset,
            // Ask for the open tier and the derived embedding readiness on every row. The incumbent
            // response carried both on the list surface (the open tier fetched per id, which is what
            // made that path an N+1; the readiness in a second read after the page) — asking for them
            // as SECTIONS gets the same answer in one statement per section for the whole page, via
            // `readback::meta_batch` and `embed_service::embedding_status_batch`. The managed tier is
            // not a section — it is always present.
            sections: Some(enriched_sections().to_csv().expect(
                "the enriched vocabulary is never empty, so it renders as a `sections` param",
            )),
            ..Default::default()
        };
    let list_result = client
        .resources()
        .list(&params)
        .await
        .across_auth(|e| match e {
            // A bare context name or invalid ref is rejected with BadRequest (spec Decision 1).
            // An unresolvable ref (not visible / not found) yields NotFound.
            // Both are caller errors → invalid_params (400-class).
            ClientError::Server {
                status: 400,
                message,
            } => rmcp::ErrorData::invalid_params(message, None),
            ClientError::NotFound { message } => {
                rmcp::ErrorData::invalid_params(format!("unknown filter: {message}"), None)
            }
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to list resources: {other}"), None)
            }
        })?;

    // The rows arrive as `ResourceView`s carrying every section the response owes — the managed
    // tier from `hit_identities`, the open tier and the derived embedding readiness from the
    // `sections` asked for above — so there is no second read and no wrapper shape: the view
    // itself is the wire answer (B1).
    let temper_workflow::types::resource::ResourceListResponse {
        rows,
        total,
        facets,
        returned,
        truncated,
        limit,
        offset,
    } = list_result;

    // Project each ROW, never the envelope: `apply_top_level_filter` keeps the named top-level keys
    // of whatever object it is given, so handing it the envelope would strip `total`/`truncated`
    // and leave the caller with a page it cannot size.
    let mut projected = Vec::with_capacity(rows.len());
    for row in &rows {
        let value = serde_json::to_value(row).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None)
        })?;
        projected.push(match input.fields.as_deref() {
            Some(fields) => temper_core::projection::apply_top_level_filter(value, fields, "id")
                .map_err(map_projection_err)?,
            None => value,
        });
    }

    let response = ListResourcesResponse {
        rows: projected,
        total,
        returned,
        truncated,
        limit,
        offset,
        facets,
    };
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&response)),
    ]))
}

pub async fn update_resource(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: UpdateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;
    // The caller-supplied managed_meta passes through untouched — identity injection and
    // slug derivation stay the door's business (the backend validation pipeline).
    let managed_meta = input.managed_meta.unwrap_or_default();
    let body = provenance_parts(input.content, input.sources, input.content_block)?;
    // Goal patch is tri-state: `goal` (set/replace, ref resolved client-side) wins over
    // `clear_goal` (retract); absent leaves the goal edge untouched.
    let (goal, clear_goal) = match (input.goal.as_deref(), input.clear_goal) {
        (Some(r), _) => (
            Some(
                temper_workflow::operations::parse_ref(r)
                    .map(|parsed| parsed.0)
                    .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?,
            ),
            None,
        ),
        (None, Some(true)) => (None, Some(true)),
        _ => (None, None),
    };

    let request = temper_workflow::types::resource::ResourceUpdateRequest {
        title: input.title.clone(),
        managed_meta: Some(managed_meta),
        open_meta: input.open_meta,
        open_meta_add: input.open_meta_add,
        content: body.content,
        content_hash: None,
        chunks_packed: None,
        context_to: None,
        type_to: None,
        goal,
        clear_goal,
        sources: body.sources,
        content_block: body.content_block,
        act: input.act,
    };

    client
        .resources()
        .update(input.id, &request)
        .await
        .across_auth(|e| match e {
            // Arm-for-arm from the direct binding: the gate's sentence names the withheld
            // capability; missing and not-modifiable share the Forbidden face on this path.
            ClientError::ForbiddenDetail { message } => {
                rmcp::ErrorData::invalid_params(message, None)
            }
            ClientError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(),
                None,
            ),
            ClientError::NotFound { message } => {
                rmcp::ErrorData::invalid_params(format!("Resource not found: {message}"), None)
            }
            // A folded content block under write addressing: the defined gone state, not a
            // server fault — the row persists as history, the address is not writable.
            ClientError::Gone { message } => rmcp::ErrorData::invalid_params(message, None),
            ClientError::Server {
                status: 400,
                message,
            } => rmcp::ErrorData::invalid_params(message, None),
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to update resource: {other}"), None)
            }
        })?;

    // Return the enriched current state, read back through the same door `get_resource` uses.
    let (enriched, _) = enriched_view(&client, input.id, false).await?;
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&enriched)),
    ]))
}

pub async fn annotate_resource(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: AnnotateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    // Classify each source (http/https URL → Remote, else ref → Resource) with the shared resolver;
    // an unparseable value is a hard error, never a silent drop.
    let sources = resolve_sources(Some(input.sources))?;
    let request = temper_workflow::types::resource::ResourceAnnotateRequest {
        sources,
        content_block: input.content_block,
        act: input.act,
    };

    client
        .resources()
        .annotate(input.id, &request)
        .await
        .across_auth(|e| match e {
            ClientError::ForbiddenDetail { message } => {
                rmcp::ErrorData::invalid_params(message, None)
            }
            ClientError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(),
                None,
            ),
            ClientError::NotFound { message } => {
                rmcp::ErrorData::invalid_params(format!("Resource not found: {message}"), None)
            }
            // A folded content block under write addressing: the defined gone state, not a
            // server fault — the row persists as history, the address is not writable.
            ClientError::Gone { message } => rmcp::ErrorData::invalid_params(message, None),
            ClientError::Server {
                status: 400,
                message,
            } => rmcp::ErrorData::invalid_params(message, None),
            other => rmcp::ErrorData::internal_error(
                format!("Failed to annotate resource: {other}"),
                None,
            ),
        })?;

    // Return the itemized provenance so the caller sees the rows it just recorded (the read that
    // proves the annotate landed), rather than re-fetching the unchanged resource body — through
    // the same door.
    let rows = client
        .resources()
        .provenance(input.id)
        .await
        .across_auth(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to read provenance: {e}"), None)
        })?;
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&rows)),
    ]))
}

pub async fn update_resource_meta(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: UpdateResourceMetaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    // The door's meta-only PUT runs the same translator's meta-only branch (body=None,
    // both meta tiers stated in full, update_meta audit, edge reconciliation). The
    // payload's resource_id/hash fields are vestigial wire baggage — the handler takes
    // the id from the path and the backend dropped caller-supplied hashes (recomputed
    // server-side, Phase 5) — so they carry named placeholders, not claims.
    let payload = temper_core::types::managed_meta::MetaUpdatePayload {
        resource_id: ResourceId::from(input.id),
        managed_meta: input.managed_meta,
        open_meta: input.open_meta,
        managed_hash: String::new(),
        open_hash: String::new(),
        act: input.act,
    };

    client
        .resources()
        .update_meta(input.id, &payload)
        .await
        .across_auth(|e| match e {
            ClientError::ForbiddenDetail { message } => {
                rmcp::ErrorData::invalid_params(message, None)
            }
            ClientError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(),
                None,
            ),
            ClientError::NotFound { message } => {
                rmcp::ErrorData::invalid_params(format!("Resource not found: {message}"), None)
            }
            ClientError::Server {
                status: 400,
                message,
            } => rmcp::ErrorData::invalid_params(message, None),
            other => rmcp::ErrorData::internal_error(
                format!("Failed to update resource meta: {other}"),
                None,
            ),
        })?;

    let response = UpdateResourceMetaResponse {
        updated: true,
        id: input.id,
    };
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&response)),
    ]))
}

pub async fn delete_resource(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: DeleteResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;

    // DELETE has no body — per-act authorship rides the query string at the door. The
    // CLI-side `force` concern stays false: the backend ignores it, as the direct
    // binding's command did.
    let response = client
        .resources()
        .delete(input.id, &input.act)
        .await
        .map(|_| ())
        // A post-edge 401 speaks its own arm FIRST (deactivated / machine gate /
        // expired-in-flight) — the family's discipline, never an internal fault with
        // a CLI login hint spoken to an MCP caller.
        .across_auth(|e| match e {
            ClientError::ForbiddenDetail { message } => {
                rmcp::ErrorData::invalid_params(message, None)
            }
            ClientError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(),
                None,
            ),
            ClientError::NotFound { message } => {
                rmcp::ErrorData::invalid_params(format!("Resource not found: {message}"), None)
            }
            // The door runs the act-authorship validation the direct binding ran
            // client-side (e.g. reasoning without a confidence band) and answers 400 —
            // a caller error, never a server fault.
            ClientError::Server {
                status: 400,
                message,
            } => rmcp::ErrorData::invalid_params(message, None),
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to delete resource: {other}"), None)
            }
        })
        .map(|_| DeleteResourceResponse {
            deleted: true,
            id: input.id,
        })?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&response)),
    ]))
}

// ── resource_grant / resource_revoke (service-direct) ──────────────
//
// Per-resource capability grants over `kb_access_grants`, mirroring the cogmap grant tool.
// Service-direct (admin events): gated by `is_system_admin OR can(...,'grant',...)` — which,
// via the owner-grant seam, includes the resource owner. NOT routed through DbBackend.

/// MCP input for resource_grant. `resource` is a ref; exactly one of `to_profile`/`to_team`
/// (raw UUID) names the principal. At least one capability must be set (read implied by write/grant).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceGrantInput {
    /// The resource, by ref (UUID or `slug-<uuid>`).
    pub resource: String,
    /// Grant to this profile (UUID). Mutually exclusive with `to_team`.
    #[serde(default)]
    pub to_profile: Option<Uuid>,
    /// Grant to this team (UUID). Mutually exclusive with `to_profile`.
    #[serde(default)]
    pub to_team: Option<Uuid>,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub grant: bool,
}

/// MCP input for resource_revoke. `resource` is a ref; exactly one of `from_profile`/`from_team`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceRevokeInput {
    pub resource: String,
    #[serde(default)]
    pub from_profile: Option<Uuid>,
    #[serde(default)]
    pub from_team: Option<Uuid>,
}

/// Resolve exactly one of (profile, team) into `(principal_table, principal_id)`.
fn resolve_grant_principal(
    profile: Option<Uuid>,
    team: Option<Uuid>,
) -> Result<(String, Uuid), rmcp::ErrorData> {
    match (profile, team) {
        (Some(p), None) => Ok(("kb_profiles".to_string(), p)),
        (None, Some(t)) => Ok(("kb_teams".to_string(), t)),
        (Some(_), Some(_)) => Err(rmcp::ErrorData::invalid_params(
            "supply exactly one principal, not both a profile and a team".to_string(),
            None,
        )),
        (None, None) => Err(rmcp::ErrorData::invalid_params(
            "no principal — supply exactly one of a profile or a team".to_string(),
            None,
        )),
    }
}

fn map_grant_error(context: &str, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{context}: caller may not administer grants on this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

/// Grant a capability on a resource, through the door. The gate is
/// `is_system_admin OR can_grant OR owner`, and it lives server-side; `read` is forced
/// on when `write`/`grant` is set.
pub async fn resource_grant(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ResourceGrantInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;
    let resource_id = uuid::Uuid::from(
        temper_workflow::operations::parse_ref(&input.resource)
            .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?,
    );
    let (principal_table, principal_id) = resolve_grant_principal(input.to_profile, input.to_team)?;
    if !(input.read || input.write || input.grant) {
        return Err(rmcp::ErrorData::invalid_params(
            "no capability selected — set at least one of read/write/grant".to_string(),
            None,
        ));
    }
    let body = temper_core::types::resource_grant::ResourceGrantBody {
        principal_table,
        principal_id,
        can_read: input.read || input.write || input.grant,
        can_write: input.write,
        can_delete: false,
        can_grant: input.grant,
    };
    let outcome = client
        .resources()
        .grant(resource_id, &body)
        .await
        .across_auth(|e| map_grant_error("resource_grant", e))?;
    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(text),
    ]))
}

/// Revoke a capability grant on a resource, through the door. Admin/can_grant/owner-gated
/// server-side. No-op safe.
pub async fn resource_revoke(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ResourceRevokeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let client = svc.relay_client(parts)?;
    let resource_id = uuid::Uuid::from(
        temper_workflow::operations::parse_ref(&input.resource)
            .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?,
    );
    let (principal_table, principal_id) =
        resolve_grant_principal(input.from_profile, input.from_team)?;
    let body = temper_core::types::resource_grant::ResourceRevokeBody {
        principal_table,
        principal_id,
    };
    let outcome = client
        .resources()
        .revoke(resource_id, &body)
        .await
        .across_auth(|e| map_grant_error("resource_revoke", e))?;
    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(text),
    ]))
}

#[cfg(test)]
mod grant_tests {
    use super::*;

    #[test]
    fn resource_grant_input_deserializes() {
        let id = Uuid::now_v7();
        let raw = serde_json::json!({ "resource": "r", "to_team": id.to_string(), "write": true });
        let input: ResourceGrantInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.to_team, Some(id));
        assert!(input.write);
        assert!(!input.grant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gap 1 regression: `managed_meta` is a typed `ManagedMeta`, so an MCP
    /// client passing a real JSON object (not a string-encoded one)
    /// deserializes straight into the typed shape.
    #[test]
    fn create_resource_input_accepts_object_valued_managed_meta() {
        let raw = serde_json::json!({
            "context_ref": "@me/demo",
            "doc_type_name": "task",
            "title": "Demo Task",
            "managed_meta": { "temper-stage": "backlog", "temper-mode": "build" },
        });
        let input: CreateResourceInput =
            serde_json::from_value(raw).expect("object-valued managed_meta must deserialize");
        let managed = input.managed_meta.expect("managed_meta present");
        assert_eq!(managed.stage.as_deref(), Some("backlog"));
        assert_eq!(managed.mode.as_deref(), Some("build"));
    }

    /// `managed_meta` is a closed vocabulary: an unknown key under it must be
    /// rejected at input parse, not silently absorbed. Proves the closed
    /// `ManagedMeta` type reaches the MCP tool boundary.
    #[test]
    fn create_input_rejects_unknown_managed_key() {
        let raw = serde_json::json!({
            "context_ref": "@me/temper",
            "doc_type_name": "task",
            "title": "T",
            "managed_meta": { "my-tag": "x" },
        });
        let err = serde_json::from_value::<CreateResourceInput>(raw).unwrap_err();
        assert!(
            err.to_string().contains("my-tag"),
            "unknown managed key must be rejected at input parse, got: {err}"
        );
    }

    #[test]
    fn update_resource_input_accepts_object_valued_managed_meta() {
        let raw = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "managed_meta": { "temper-stage": "done" },
        });
        let input: UpdateResourceInput =
            serde_json::from_value(raw).expect("object-valued managed_meta must deserialize");
        assert_eq!(
            input
                .managed_meta
                .expect("managed_meta present")
                .stage
                .as_deref(),
            Some("done"),
        );
    }

    /// The non-authored MCP write inputs (update / delete) accept the same flattened act fields, so
    /// an agent can correlate + author an update/delete the same way it does a create.
    #[test]
    fn update_resource_input_accepts_act_authorship_fields() {
        let raw = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "open_meta": { "reviewed_by": "qa" },
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "applying review outcome",
            "confidence": "confident",
        });
        let input: UpdateResourceInput =
            serde_json::from_value(raw).expect("flattened act fields must deserialize");
        assert!(input.act.invocation_id.is_some());
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Confident)
        );
        assert!(!input.act.into_act_context().expect("assembles").is_empty());
    }

    #[test]
    fn delete_resource_input_accepts_act_authorship_fields() {
        let raw = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "reasoning": "tombstoning the duplicate",
            "confidence": "tentative",
        });
        let input: DeleteResourceInput =
            serde_json::from_value(raw).expect("flattened act fields must deserialize");
        assert_eq!(
            input.act.reasoning.as_deref(),
            Some("tombstoning the duplicate")
        );
        assert!(!input.act.into_act_context().expect("assembles").is_empty());
    }

    /// Chunk B: the flattened [`ActInput`] discrete fields deserialize as top-level keys on the
    /// MCP input (invocation_id + the authorship fields), and assemble into an `ActContext`.
    #[test]
    fn create_resource_input_accepts_act_authorship_fields() {
        let raw = serde_json::json!({
            "context_ref": "@me/demo",
            "doc_type_name": "task",
            "title": "Demo Task",
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "seeding the demo corpus",
            "confidence": "probable",
            "persona": "steward",
        });
        let input: CreateResourceInput =
            serde_json::from_value(raw).expect("flattened act fields must deserialize");
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Probable)
        );
        assert_eq!(
            input.act.reasoning.as_deref(),
            Some("seeding the demo corpus")
        );
        assert_eq!(input.act.persona.as_deref(), Some("steward"));
        assert!(input.act.invocation_id.is_some(), "invocation_id present");
        // And it assembles into a non-empty ActContext.
        let ctx = input.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }

    /// Chunk B: the flattened authorship/correlation fields must inline as a string enum
    /// (`confidence`) and a string-uuid (`invocation_id`) in the generated schema — a `$ref` into
    /// `$defs` reaches the Anthropic tool-use layer with no type signal and comes back as `null`
    /// (the same bug fixed for EdgeKind/Polarity). Generated via the exact rmcp runtime path.
    #[test]
    fn create_resource_input_schema_inlines_act_fields() {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        let schema = serde_json::to_value(generator.into_root_schema_for::<CreateResourceInput>())
            .expect("schema serializes");

        // confidence: inline string enum (the trailing `null` is the field's Option-ness).
        let confidence = &schema["properties"]["confidence"];
        assert!(
            confidence.get("$ref").is_none(),
            "confidence must inline, not $ref: {confidence}"
        );
        let variants: Vec<&str> = confidence
            .get("enum")
            .and_then(|e| e.as_array())
            .expect("confidence carries inline enum variants")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(variants, ["tentative", "probable", "confident"]);

        // invocation_id: inline string-uuid, not a $ref into $defs.
        let invocation = &schema["properties"]["invocation_id"];
        assert!(
            invocation.get("$ref").is_none(),
            "invocation_id must inline, not $ref: {invocation}"
        );
        assert_eq!(
            invocation.get("format").and_then(|f| f.as_str()),
            Some("uuid"),
            "invocation_id inlines as a uuid-format string: {invocation}"
        );

        // correlation_id: same contract. A caller-minted act-grain thread is useless if the tool
        // layer sees `null` where a uuid should be.
        let correlation = &schema["properties"]["correlation_id"];
        assert!(
            correlation.get("$ref").is_none(),
            "correlation_id must inline, not $ref: {correlation}"
        );
        assert_eq!(
            correlation.get("format").and_then(|f| f.as_str()),
            Some("uuid"),
            "correlation_id inlines as a uuid-format string: {correlation}"
        );
        assert!(
            !schema.to_string().contains("CorrelationId"),
            "CorrelationId must not survive as a named $defs entry: {schema}"
        );
    }

    /// Gap 1: the generated JsonSchema must describe `managed_meta` as the
    /// concrete `ManagedMeta` object rather than free-form JSON — that
    /// concreteness is what stops MCP clients from string-encoding the field.
    /// Under the self-contained-declarations rule the concreteness is INLINE:
    /// the property advertises `type: object` with named properties and no
    /// `$ref`. Before the rule the property `$ref`'d a `$defs` entry and this
    /// test keyed on the type name appearing in the schema string.
    #[test]
    fn create_resource_input_managed_meta_schema_is_concrete() {
        let schema = serde_json::to_value(schemars::schema_for!(CreateResourceInput))
            .expect("schema serializes");
        let managed = &schema["properties"]["managed_meta"];
        assert!(
            managed.get("$ref").is_none(),
            "managed_meta must be inlined, not a $ref: {managed}"
        );
        let is_object = match &managed["type"] {
            serde_json::Value::String(s) => s == "object",
            serde_json::Value::Array(items) => items.iter().any(|v| v.as_str() == Some("object")),
            _ => false,
        };
        assert!(
            is_object,
            "managed_meta must advertise type object: {managed}"
        );
        assert!(
            managed["properties"].is_object(),
            "managed_meta must carry its named properties inline, not free-form: {managed}"
        );
    }
}

#[cfg(test)]
mod enriched_resource_tests {
    use super::*;

    /// A view as the read paths hand one over: `with_derived_refs` has run, so `ref` and
    /// `context_ref` are filled from the columns beside them rather than by this fixture.
    pub(super) fn sample_view() -> ResourceView {
        use temper_core::types::ids::ContextId;
        let nil = uuid::Uuid::nil();
        ResourceView {
            id: ResourceId::from(uuid::Uuid::now_v7()),
            r#ref: String::new(),
            title: "Wire the widget".to_string(),
            origin_uri: "temper://fixture/task-doc".to_string(),
            kb_context_id: Some(ContextId::from(nil)),
            context_name: Some("temper".to_string()),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "task".to_string(),
            owner_handle: "me".to_string(),
            owner_profile_id: ProfileId::from(nil),
            originator_profile_id: ProfileId::from(nil),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            body_hash: None,
            ingest_state: Some(temper_core::types::resource::IngestState::Complete),
            body_storage: Some(temper_core::types::resource::BodyStorage::Derived),
            managed_meta: ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            content: None,
            embedding_status: None,
        }
        .with_derived_refs()
    }

    /// The MCP response is the view itself, the `embedding-status` section asked for.
    ///
    /// B1 dissolved the flattened `EnrichedResource` wrapper into the view that carries the
    /// field, so the wire answer is the door's own `ResourceView` with `embedding_status`
    /// filled — the same keys at the same depth the wrapper used to emit, which is what keeps
    /// `fields` projection (TOP-LEVEL keys anchored on `id`) working unchanged.
    #[test]
    fn the_mcp_response_is_the_view_with_the_section_asked_for() {
        let mut view = sample_view();
        let expected_ref = view.r#ref.clone();

        // Not requested: the key is absent — never a null, never a fourth state.
        let not_asked = serde_json::to_value(&view).expect("serialize view");
        assert!(
            !not_asked
                .as_object()
                .expect("object")
                .contains_key("embedding_status"),
            "an unasked section omits the key: {not_asked}"
        );

        // Requested (what `enriched_view` always asks): the key rides at the top level,
        // beside the view's own keys.
        view.embedding_status = Some(EmbeddingStatus::Ready);
        let answered = serde_json::to_value(&view).expect("serialize view");
        assert_eq!(answered["embedding_status"], "ready");

        // `ref` is the affordance this task exists to deliver to MCP callers. It is the decorated
        // form, derived once by `with_derived_refs`, never re-derived at render time.
        assert_eq!(
            answered["ref"].as_str(),
            Some(expected_ref.as_str()),
            "every MCP resource response carries a decorated ref: {answered}"
        );
        assert_eq!(
            expected_ref,
            temper_core::refs::decorated_ref(&view.title, view.id)
        );
    }

    /// The four workflow values a caller used to read as flat columns are still reachable — one
    /// level down, under their canonical `temper-*` names. Dropping the hoist was lossless on this
    /// surface too, not only on the HTTP one.
    #[test]
    fn workflow_metadata_reaches_the_mcp_caller_under_managed_meta() {
        let view = ResourceView {
            embedding_status: Some(EmbeddingStatus::Ready),
            ..sample_view()
        };
        let v = serde_json::to_value(&view).expect("serialize");

        assert_eq!(v["managed_meta"]["temper-stage"], "in-progress");
        assert!(
            v.as_object().expect("object").get("stage").is_none(),
            "`stage` is not hoisted onto the MCP response either: {v}"
        );
    }

    /// The list envelope carries the paging state the shipped agent skill tells agents to read.
    ///
    /// `returned` and `truncated` are not recomputed here — they are carried through from
    /// `ResourceListResponse::new`, which is the one place the `offset + returned < total`
    /// derivation lives. This asserts the carry, not a second derivation.
    #[test]
    fn list_envelope_carries_returned_and_truncated() {
        use temper_workflow::types::resource::{ResourceFacets, ResourceListResponse};

        let page = ResourceListResponse::new(
            vec![sample_view()],
            25,
            ResourceFacets::default(),
            Some(1),
            0,
        );
        let response = ListResourcesResponse {
            rows: vec![serde_json::json!({})],
            total: page.total,
            returned: page.returned,
            truncated: page.truncated,
            limit: page.limit,
            offset: page.offset,
            facets: page.facets,
        };

        let v = serde_json::to_value(&response).expect("serialize");
        assert_eq!(v["total"], 25);
        assert_eq!(v["returned"], 1);
        assert_eq!(
            v["truncated"], true,
            "24 of 25 matching rows are beyond this page: {v}"
        );
        assert_eq!(v["limit"], 1);
        assert_eq!(v["offset"], 0);
    }
}

#[cfg(test)]
mod fields_projection_tests {
    use super::*;

    #[test]
    fn get_resource_input_is_ref_only() {
        let raw = serde_json::json!({ "id": "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2" });
        let input: GetResourceInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.id, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
    }

    #[test]
    fn get_resource_input_accepts_fields() {
        // Compile-time check that GetResourceInput carries the field.
        let _input = GetResourceInput {
            id: "x".to_string(),
            include_content: Some(false),
            fields: Some(vec!["managed_meta".to_string()]),
        };
    }

    /// Projection over a REAL serialized response, not a hand-written stub.
    ///
    /// The stub this replaced still listed `slug` and `owner`, keys the shape has not carried for
    /// some time — a projection test written against an invented object cannot notice that the
    /// object it projects has stopped existing. Serializing the actual type is what ties this
    /// assertion to the wire.
    #[test]
    fn enriched_resource_filtered_by_fields_preserves_id_and_managed_meta() {
        let view = ResourceView {
            embedding_status: Some(EmbeddingStatus::Ready),
            ..super::enriched_resource_tests::sample_view()
        };
        let value = serde_json::to_value(&view).expect("serialize");
        let filtered = temper_core::projection::apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "id",
        )
        .expect("filter");

        assert!(filtered.get("id").is_some(), "anchor id missing");
        assert_eq!(filtered["managed_meta"]["temper-stage"], "in-progress");
        assert!(filtered.get("title").is_none(), "title should be dropped");
        assert!(
            filtered.get("ref").is_none(),
            "an unnamed field is dropped even when it is the one this task added"
        );
    }

    #[test]
    fn list_resources_input_accepts_fields() {
        // Compile-time check that ListResourcesInput grows the fields field.
        let _input = ListResourcesInput {
            context_ref: None,
            doc_type_name: None,
            stage: None,
            status: None,
            tags: None,
            goal: None,
            cogmap: None,
            limit: None,
            offset: None,
            fields: Some(vec!["managed_meta".to_string()]),
        };
    }

    /// MCP can filter by `stage` and `status`, not only by goal and cogmap.
    ///
    /// Until 2026-07-28 `ListResourcesInput` carried neither, so the door agents come
    /// through could not narrow a task list by stage at all while the CLI and API could —
    /// a parity gap discoverable only by an agent attempting it and getting everything
    /// back. Constructed via the deserializer rather than a struct literal so this also
    /// pins the wire names an MCP caller actually sends.
    #[test]
    fn list_resources_input_accepts_stage_and_status() {
        let input: ListResourcesInput = serde_json::from_value(serde_json::json!({
            "doc_type_name": "goal",
            "stage": "in-progress",
            "status": "active",
        }))
        .expect("stage and status must deserialize from the MCP wire shape");
        assert_eq!(input.stage.as_deref(), Some("in-progress"));
        assert_eq!(input.status.as_deref(), Some("active"));
    }

    /// MCP takes `tags` as a LIST, not the CSV the HTTP layer carries.
    ///
    /// The wire shapes diverge on purpose: the list endpoint is a GET whose params ride the
    /// query string (serde_urlencoded encodes no sequences), so the CSV is a transport
    /// constraint of *that* door — not a shape to propagate to a JSON-RPC caller that can send
    /// an array natively. The join to CSV happens inside the tool. Constructed via the
    /// deserializer so this pins the wire name an MCP caller actually sends.
    #[test]
    fn list_resources_input_accepts_a_tag_list() {
        let input: ListResourcesInput = serde_json::from_value(serde_json::json!({
            "tags": ["ci", "security"],
        }))
        .expect("tags must deserialize as a list from the MCP wire shape");
        assert_eq!(
            input.tags.as_deref(),
            Some(["ci".to_string(), "security".to_string()].as_slice())
        );
        // And no doc_type_name is required alongside it — tags are not doc-type-scoped, which
        // is what lets one call enumerate an axis that spans 14 doc types.
        assert_eq!(input.doc_type_name, None);
    }

    #[test]
    fn enriched_resource_carries_decorated_ref() {
        let id = uuid::Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap();
        let got = temper_workflow::operations::decorated_ref(
            "My Task",
            temper_core::types::ids::ResourceId(id),
        );
        assert_eq!(got, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
    }

    #[test]
    fn enriched_resource_array_filtered_by_fields() {
        let value = serde_json::json!([
            {
                "id": "11111111-1111-1111-1111-111111111111",
                "title": "A",
                "managed_meta": {"stage": "done"}
            },
            {
                "id": "22222222-2222-2222-2222-222222222222",
                "title": "B",
                "managed_meta": {"stage": "in-progress"}
            }
        ]);
        let filtered = temper_core::projection::apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "id",
        )
        .expect("filter");
        let arr = filtered.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for row in arr {
            assert!(row.get("id").is_some());
            assert!(row.get("managed_meta").is_some());
            assert!(row.get("title").is_none());
        }
    }
}
