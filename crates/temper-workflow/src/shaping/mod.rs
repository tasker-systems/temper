//! Request shaping for the resource writes: one function per act, shared by every door that
//! builds a write request (the CLI and MCP).
//!
//! Each function maps a door-neutral input onto the wire request its act sends, and owns every
//! default that mapping applies. So the same inputs produce the same request, and the same stored
//! result, whichever door carried them.
//!
//! Shaping refuses nothing. The server is the only enforcing copy of the rules a write must
//! satisfy; a door keeps only its own parse errors (a ref that does not parse, a flag combination
//! its own syntax forbids). The one failure here is a value that will not serialize.
//!
//! Embedding is not shaping: a door that embeds (the CLI) fills `content_hash` and
//! `chunks_packed` from the shaped `content` afterwards, and only when that content is non-empty.
//!
//! Delete has no shaping function: its whole request is the caller's [`ActInput`], sent as it is.

use serde_json::Value;
use temper_core::types::authorship::ActInput;
use temper_core::types::ingest::{IngestPayload, SegmentedBegin};
use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::precondition::ResourceExpect;
use temper_core::types::provenance::ProvenanceSource;
use temper_core::types::resource::{ResourceAnnotateRequest, ResourceUpdateRequest};
use uuid::Uuid;

/// The only way shaping fails: an input that will not serialize to its wire form.
#[derive(Debug, thiserror::Error)]
pub enum ShapeError {
    /// `managed_meta` could not be rendered as JSON.
    #[error("serialize managed_meta: {0}")]
    ManagedMeta(String),
}

/// Where a created resource lives: exactly one home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Home {
    /// A context, by the caller's ref (`@owner/slug`, `+team/slug` or a UUID), sent as written.
    /// The server parses and resolves it.
    Context(String),
    /// A cognitive map, by id.
    Cogmap(Uuid),
}

/// A create, as every door expresses it.
#[derive(Debug, Clone)]
pub struct CreateInput {
    pub title: String,
    pub doc_type: String,
    pub home: Home,
    /// The body. `None` and an empty string both mean no body; the resource is created with none.
    pub content: Option<String>,
    /// Block-provenance sources for the body.
    pub sources: Vec<ProvenanceSource>,
    pub managed_meta: Option<ManagedMeta>,
    pub open_meta: Option<Value>,
    /// The goal this resource advances, already resolved to its id.
    pub goal: Option<Uuid>,
    /// The resource's origin. Absent: none (`""`).
    pub origin_uri: Option<String>,
    /// The create's idempotency key. Absent: one is minted, so every create can be retried safely.
    pub idempotency_key: Option<Uuid>,
    pub act: ActInput,
}

/// How an update changes the resource's goal edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalChange {
    /// Point the resource at this goal, replacing any current one.
    Set(Uuid),
    /// Retract the current goal edge.
    Clear,
}

/// An update (`PATCH /api/resources/{id}`), as every door expresses it. Every field is a partial
/// patch: absent leaves the stored value alone.
#[derive(Debug, Clone, Default)]
pub struct UpdateInput {
    pub title: Option<String>,
    pub managed_meta: Option<ManagedMeta>,
    pub open_meta: Option<Value>,
    pub open_meta_add: Option<Value>,
    /// A body revise. An empty string means no revise, the same as `None`.
    pub content: Option<String>,
    pub sources: Vec<ProvenanceSource>,
    pub content_block: Option<Uuid>,
    /// Move the resource to this context, by the caller's ref, sent as written.
    pub context_to: Option<String>,
    /// Convert the resource to this doc type, by name.
    pub type_to: Option<String>,
    pub goal: Option<GoalChange>,
    pub expect: Option<ResourceExpect>,
    pub act: ActInput,
}

/// An annotate: provenance sources attached to a block without a body revise.
#[derive(Debug, Clone)]
pub struct AnnotateInput {
    pub sources: Vec<ProvenanceSource>,
    pub content_block: Option<Uuid>,
    pub act: ActInput,
}

/// Shape a one-shot create (`POST /api/ingest`).
pub fn shape_create(input: CreateInput) -> Result<IngestPayload, ShapeError> {
    let managed_meta = managed_meta_value(input.managed_meta)?;
    let (context_ref, home_cogmap_id) = match input.home {
        Home::Context(context_ref) => (context_ref, None),
        Home::Cogmap(id) => (String::new(), Some(id)),
    };
    Ok(IngestPayload {
        title: input.title,
        origin_uri: input.origin_uri.unwrap_or_default(),
        context_ref,
        home_cogmap_id,
        doc_type_name: input.doc_type,
        goal: input.goal,
        content_hash: None,
        idempotency_key: Some(input.idempotency_key.unwrap_or_else(Uuid::now_v7)),
        content: input.content.unwrap_or_default(),
        metadata: None,
        managed_meta,
        open_meta: input.open_meta,
        chunks_packed: None,
        sources: input.sources,
        act: input.act,
        segmented: None,
    })
}

/// Shape the begin of a segmented create (`POST /api/ingest` with `segmented` set). The input's
/// `content` is segment 0's text, not the whole body; every other field is shaped exactly as
/// [`shape_create`] shapes it.
pub fn shape_segmented_begin(
    input: CreateInput,
    segmented: SegmentedBegin,
) -> Result<IngestPayload, ShapeError> {
    let mut payload = shape_create(input)?;
    payload.segmented = Some(segmented);
    Ok(payload)
}

/// Shape an update (`PATCH /api/resources/{id}`).
pub fn shape_update(input: UpdateInput) -> Result<ResourceUpdateRequest, ShapeError> {
    let (goal, clear_goal) = match input.goal {
        Some(GoalChange::Set(id)) => (Some(id), None),
        Some(GoalChange::Clear) => (None, Some(true)),
        None => (None, None),
    };
    Ok(ResourceUpdateRequest {
        title: input.title,
        managed_meta: input.managed_meta.filter(|m| *m != ManagedMeta::default()),
        open_meta: input.open_meta,
        open_meta_add: input.open_meta_add,
        content: input.content.filter(|c| !c.is_empty()),
        content_hash: None,
        chunks_packed: None,
        context_to: input.context_to,
        type_to: input.type_to,
        goal,
        clear_goal,
        sources: input.sources,
        content_block: input.content_block,
        expect: input.expect,
        act: input.act,
    })
}

/// Shape an annotate (`POST /api/resources/{id}/annotate`).
pub fn shape_annotate(input: AnnotateInput) -> ResourceAnnotateRequest {
    ResourceAnnotateRequest {
        sources: input.sources,
        content_block: input.content_block,
        act: input.act,
    }
}

/// A create's managed tier on the wire: absent when there is nothing in it.
fn managed_meta_value(managed_meta: Option<ManagedMeta>) -> Result<Option<Value>, ShapeError> {
    managed_meta
        .filter(|m| *m != ManagedMeta::default())
        .map(|m| serde_json::to_value(m).map_err(|e| ShapeError::ManagedMeta(e.to_string())))
        .transpose()
}

#[cfg(test)]
mod tests;
