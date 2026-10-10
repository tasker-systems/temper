//! The erasure family's wire shapes — principal erasure, resource erasure, the block-history
//! scrub and the field scrub: the doors' requests and answers, and the ledger fragments they carry.
//!
//! These are temper-core's because they are wire types (every wire type lives here, so every
//! client can name them without depending on the server crates). `temper_substrate::payloads`
//! re-exports the ones the ledger's event payloads embed, the way `temper_substrate::ids`
//! re-exports the id newtypes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::{BlobId, EdgeId, EventId, ProfileId, PropertyId, ResourceId};

/// One target of a completed erasure and what happened to it (erasure spec, "per-target
/// outcomes"). The target names itself the way the personal-data manifest does — `table` or
/// `table.column`; the outcome is the act's own record of what redaction applied. Deliberately
/// open-textured in v1: ceilings are DATA, not types (D1), and the per-target vocabulary is the
/// execution build's to pin. `unhonourable_scope` outcomes land here, never silent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ErasureTargetOutcome {
    /// Manifest identity of the target (`kb_profiles.display_name`, `kb_teams.slug`, …).
    pub target: String,
    /// What the act did to it (erased / sentinel-scrubbed / accepted-in-part / …).
    pub outcome: String,
}

/// The ledger paths of one event: redacted (`redacted_fields`) or named-and-unreached
/// (`ledger_remainder`). ONE shape for both, so the cut-2 completion pass derives what it redacts
/// from what cut 1 recorded without translating (resource erasure spec D12). The event is keyed
/// `event`, never `event_id` — no trail join-key shape rides an admin payload (D1). Paths only,
/// never values: the record of a redaction must not carry what was redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct RedactedEventFields {
    pub event: EventId,
    /// JSON paths within that event's `payload` (or `metadata`), e.g. `title`, `origin_uri`.
    pub paths: Vec<String>,
}

// The erasure and block history scrub doors answer in this enum. The ledger records refusals in
// `RecordedRefusalReason`, which carries every value here and the field scrub's.
/// The closed refusal vocabulary for `resource_erasure_refused` (resource erasure spec D5, D11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResourceErasureRefusalReason {
    /// Retired: no path raises it. A non-admin is refused at the wire with no event. The value
    /// stays registered because removing one from a closed vocabulary is not additive.
    Unauthorized,
    /// A cogmap's telos/charter resource: map-grain erasure is its own act, named in `detail`.
    CharterResource,
    /// Retired: no path raises it. Ingest state does not refuse an erasure; an in-flight ingest
    /// ends with it (spec D5). The value stays registered because removing one from a closed
    /// vocabulary is not additive.
    IngestInFlight,
    /// The resource is already erased. Recorded by the erasure act on a repeat request and by
    /// the block history scrub, which has nothing to scrub on an erased resource. Nothing in the
    /// projection changes and no second `resource_erased` is minted.
    AlreadyErased,
}

/// The refusal vocabulary the field scrub's doors answer with (field-grain scrub spec S4). Its own
/// type, not new values of [`ResourceErasureRefusalReason`]: those doors' clients parse that enum,
/// and a value they never saw would strand them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FieldScrubRefusalReason {
    /// A cogmap's telos/charter resource, as for the erasure.
    CharterResource,
    /// The resource is already erased: there is nothing left to scrub.
    AlreadyErased,
    /// The owner's ledger already names the text of one of the field scrub's sentinels. Writing
    /// it could only make replay diverge, so the field scrub records this and changes nothing.
    SentinelCollision,
    /// In keep mode, the latest event carrying the title or origin URI disagrees with the
    /// resource's projected value: an inversion inside a transaction, which the field scrub
    /// refuses rather than resolves. Nothing changes.
    ProjectionDisagrees,
}

/// The closed refusal vocabulary `resource_erasure_refused` records (resource erasure spec D5,
/// D11; field-grain scrub spec S4): every value either act family's doors answer with. A ledger
/// vocabulary only; no door answers in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RecordedRefusalReason {
    /// Retired: no path raises it. The value stays registered because removing one from a closed
    /// vocabulary is not additive.
    Unauthorized,
    /// A cogmap's telos/charter resource: map-grain erasure is its own act, named in `detail`.
    CharterResource,
    /// Retired: no path raises it. The value stays registered because removing one from a closed
    /// vocabulary is not additive.
    IngestInFlight,
    /// The resource is already erased. Nothing in the projection changes and no second
    /// `resource_erased` is minted.
    AlreadyErased,
    /// The field scrub's sentinel text is already on the owner's ledger.
    SentinelCollision,
    /// The field scrub found the latest title or origin URI event disagreeing with the projection.
    ProjectionDisagrees,
}

impl From<ResourceErasureRefusalReason> for RecordedRefusalReason {
    fn from(reason: ResourceErasureRefusalReason) -> Self {
        match reason {
            ResourceErasureRefusalReason::Unauthorized => Self::Unauthorized,
            ResourceErasureRefusalReason::CharterResource => Self::CharterResource,
            ResourceErasureRefusalReason::IngestInFlight => Self::IngestInFlight,
            ResourceErasureRefusalReason::AlreadyErased => Self::AlreadyErased,
        }
    }
}

impl From<FieldScrubRefusalReason> for RecordedRefusalReason {
    fn from(reason: FieldScrubRefusalReason) -> Self {
        match reason {
            FieldScrubRefusalReason::CharterResource => Self::CharterResource,
            FieldScrubRefusalReason::AlreadyErased => Self::AlreadyErased,
            FieldScrubRefusalReason::SentinelCollision => Self::SentinelCollision,
            FieldScrubRefusalReason::ProjectionDisagrees => Self::ProjectionDisagrees,
        }
    }
}

/// Which act a `resource_erasure_refused` event refuses. Absent on the payload reads as
/// [`ErasureAct::Erasure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ErasureAct {
    /// The resource erasure act.
    Erasure,
    /// The block history scrub.
    BlockHistoryScrub,
    /// The field scrub: a resource's title, origin URI or property history redacted while the
    /// resource survives.
    FieldScrub,
}

/// The field a field scrub names. A field, never a value: no request or record carries the text
/// being scrubbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ScrubFieldKind {
    /// The resource's title.
    Title,
    /// The resource's origin URI.
    OriginUri,
    /// One resource-owned property family, named by its handle.
    Property,
    /// Every resource-owned property family's prior history. Never cleared.
    Properties,
}

/// The field a field scrub acted on. `family` is the handle of a property family: the id of the
/// first event that still carries the family's key text on the resource. It is present only for
/// [`ScrubFieldKind::Property`]. Keyed `family`, never `event_id`: no trail join-key shape rides
/// an admin payload (resource erasure D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ScrubbedField {
    pub kind: ScrubFieldKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<EventId>,
}

/// An edge touching the resource whose asserting principal is not the resource's owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct OtherAuthorEdge {
    pub edge_id: EdgeId,
    /// The profile behind the entity that emitted the edge's asserting event.
    pub author: ProfileId,
    /// Already folded at survey time: the act appends no fold for it, but still nulls its label
    /// and sentinels its properties (steps 9c and 9d reach live and folded edges).
    pub folded: bool,
}

/// A property row owned by an edge touching the resource, asserted by another principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct OtherAuthorEdgeProperty {
    pub property_id: PropertyId,
    pub edge_id: EdgeId,
    pub author: ProfileId,
    /// Already folded at survey time (the row's own fold): the act still sentinels its key and
    /// value (step 9d reaches live and folded rows).
    pub folded: bool,
}

/// A related blob and the other resources that hold a live edge to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlobCoLinks {
    pub blob_id: BlobId,
    /// Empty when no other resource links the blob.
    pub holders: Vec<ResourceId>,
}

/// Whether the sensitivity sweep has confirmed that a deriver quotes one of the erased resource's
/// detected values (resource erasure spec D10, as amended by the build order 3c rulings). Read
/// from stored fingerprints, never from content; the act never reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FingerprintMatch {
    /// The deriver still holds a value the sweep found in the resource.
    Yes,
    /// The sweep has read all of both, and the deriver holds none of the resource's detected
    /// values. Never a claim that the deriver is clean: only what the detectors recognise.
    No,
    /// The sweep has not read all of the resource or the deriver: it is off, a detector is
    /// disabled, or it has not reached them yet.
    Unscanned,
    /// A fingerprint the comparison needs is gone (30 days after an erasure act emptied its
    /// place), capped, or never minted. No later sweep can make `no` trustworthy: read the deriver.
    Expired,
}

/// One deriver the survey names (D8), with its [`FingerprintMatch`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeriverFingerprint {
    pub deriver: ResourceId,
    pub fingerprint_match: FingerprintMatch,
}

/// The plan `resource_erasure_survey` renders, plus the display-only annotations. The act never
/// consumes the annotations (D10's fingerprint posture): they are read after the plan, in Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceErasurePlan {
    pub n_blocks: i64,
    pub n_revisions: i64,
    pub n_chunks: i64,
    pub n_artifacts: i64,
    pub n_edges: i64,
    /// The live edges the act would fold.
    pub edges: Vec<EdgeId>,
    pub targets: Vec<ErasureTargetOutcome>,
    /// Set when the resource is a cogmap's charter: the act would refuse.
    pub charter_of: Option<Uuid>,
    pub ingest_state: String,
    pub fingerprint_available: bool,
    /// Derivers, related blobs, cross-resource ledger text and shared remote sources (D8).
    pub remainder: Vec<ErasureTargetOutcome>,
    /// Each deriver the remainder names, in its order, with whether the sweep confirms it quotes
    /// one of the resource's detected values (D10). Empty from a server that predates it.
    #[serde(default)]
    pub deriver_fingerprints: Vec<DeriverFingerprint>,
    /// The resource's own ledger paths the act would rewrite to their sentinels (D3). Empty from a
    /// server that predates the ledger exception.
    #[serde(default)]
    pub redacted_fields: Vec<RedactedEventFields>,
    /// The ledger paths carrying the resource's content that the act cannot reach (D12).
    pub ledger_remainder: Vec<RedactedEventFields>,
    pub other_author_edges: Vec<OtherAuthorEdge>,
    pub other_author_edge_properties: Vec<OtherAuthorEdgeProperty>,
    pub blob_co_links: Vec<BlobCoLinks>,
}

/// The read-only survey. `plan` is `None` exactly when the resource was already erased when the
/// survey began (the short-circuit); `already_erased` also reads true when an act lands between
/// that read and the plan, and then the plan is present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceErasureSurvey {
    pub resource: ResourceId,
    pub already_erased: bool,
    pub plan: Option<ResourceErasurePlan>,
    /// On an erased resource (the short-circuit), the ledger paths a completion pass would rewrite
    /// now (D12): a resource erased before the ledger exception shipped still carries its text
    /// there, and running the act again completes it. Empty when nothing is left, and then the act
    /// refuses `already_erased`; empty too from a server that predates the completion pass.
    #[serde(default)]
    pub completion_fields: Vec<RedactedEventFields>,
    /// On an erased resource, the derivers its erasure named, with whether the sweep confirms each
    /// quotes one of its detected values (D10, sweep D11): the fingerprints stay comparable for 30
    /// days after the act, then read `expired`. Before the act the annotation is on `plan`. Empty
    /// from a server that predates it.
    #[serde(default)]
    pub deriver_fingerprints: Vec<DeriverFingerprint>,
}

/// One named block in the plan: what the scrub would empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlockScrubCount {
    pub block: Uuid,
    /// A folded block empties entirely; a live block keeps its current revision and chunks.
    pub folded: bool,
    pub revisions_to_empty: i64,
    pub chunks_to_empty: i64,
    /// The sensitivity sweep holds an open finding on the block's current revision or its current
    /// chunks: the text has not been edited out yet, and the scrub will keep it (D11). False for a
    /// current revision the sweep has not read: it confirms a leak, never cleanliness. Absent from a
    /// server that predates it.
    #[serde(default)]
    pub current_revision_flagged: bool,
}

/// The plan `block_history_scrub_survey` renders (D10: the act consumes the same computation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlockHistoryScrubPlan {
    /// True when the scrub would cancel an in-flight ingest.
    pub cancels_ingest: bool,
    /// Per named block, in the operator's order.
    pub blocks: Vec<BlockScrubCount>,
}

/// The read-only survey. Exactly one of `refusal` and `plan` is present: `refusal` when the act
/// would refuse (a charter, or an already-erased resource), with `detail` for a charter;
/// otherwise the per-block `plan`. Nothing is recorded either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlockHistoryScrubSurvey {
    pub resource: ResourceId,
    /// The refusal the act would record now.
    pub refusal: Option<ResourceErasureRefusalReason>,
    /// The refusal's fixed evidence (a charter's map-grain task).
    pub detail: Option<String>,
    pub plan: Option<BlockHistoryScrubPlan>,
}

/// One blob strike of a completed erasure, as the door reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlobStrikeView {
    pub blob_id: Uuid,
    pub released: bool,
}

/// The survey door's request: the subject as the pseudonym UUID, and nothing else. No
/// request_reference — nothing is requested (ruled 2026-09-12: a survey attempt is not an
/// erasure request, so no reference is minted and no refusal would be recorded).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ErasureSurveyRequest {
    pub subject: Uuid,
}

/// The execute door's request: the subject as the pseudonym UUID, plus the opaque request
/// reference (UUID — the `RefRel::Request` apparatus Beat 2 pinned). No name, no email, no case
/// description: the request-to-person mapping lives in the operator's DSAR records, outside the
/// ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ErasureExecuteRequest {
    pub subject: Uuid,
    pub request_reference: Uuid,
}

/// What the survey predicts the act would do — the execute response minus `event_id`: the
/// survey fires no event, so there is no event id to report. The targets are the prose the
/// act would write; the blob strikes are PREDICTIONS honest about the moment the survey ran
/// (the act's strike-time verdict is authoritative).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ErasureSurveyResponse {
    pub subject: Uuid,
    pub already_erased: bool,
    /// The redacted set (D2) the act would admit.
    pub redacted_hashes: Vec<String>,
    /// Per-target outcomes and the named remainder, exactly as the record would carry them.
    pub targets: Vec<ErasureTargetOutcome>,
    pub blob_strikes: Vec<BlobStrikeView>,
}

/// What the door's act did: the completion, in full or as the no-op completion on an
/// already-erased subject. It is the door's only answer, since no door raises a principal refusal
/// (a caller who is not a system admin is answered 404 before dispatch). It stays a tagged enum
/// of one variant so the wire keeps `"status": "completed"`: removing the tag would change the
/// body's shape for no behavioural reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ErasureExecuteResponse {
    Completed {
        event_id: Uuid,
        already_erased: bool,
        /// The redacted set (D2): content hashes only.
        redacted_hashes: Vec<String>,
        /// Per-target outcomes and the named remainder (D6's accepted-in-part arm): the
        /// operator sees the `independent_obligation` remainder AT THE DOOR, not only in the
        /// ledger — the completion's own payload is the audit, but the door's caller is the
        /// actor and deserves the same facts.
        targets: Vec<ErasureTargetOutcome>,
        blob_strikes: Vec<BlobStrikeView>,
    },
}

/// The survey door's request: the resource and nothing else (a survey requests nothing).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceErasureSurveyRequest {
    pub resource: Uuid,
}

/// The execute door's request. `deny_unknown_fields`: the act's request reference is minted by
/// the service, so a caller that sends one is refused, not ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ResourceErasureExecuteRequest {
    pub resource: Uuid,
    /// Related blobs to strike with the resource (D8); each must be in the survey's remainder.
    pub also_strike_blobs: Option<Vec<Uuid>>,
}

/// What the execute door's act did: a completion and a refusal are different answers, so the
/// response is a tagged enum. A refusal here is an operator-facing one (`charter_resource`,
/// `already_erased`); a caller who is not a system admin never reaches the act.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResourceErasureExecuteResponse {
    Completed {
        /// The server-minted reference the operator cites.
        request_reference: Uuid,
        event_id: Uuid,
        folded_edges: Vec<EdgeId>,
        targets: Vec<ErasureTargetOutcome>,
        remainder: Vec<ErasureTargetOutcome>,
        /// The ledger paths the act rewrote to their sentinels (D3). Empty from a server that
        /// predates the ledger exception.
        #[serde(default)]
        redacted_fields: Vec<RedactedEventFields>,
        ledger_remainder: Vec<RedactedEventFields>,
        blob_strikes: Vec<BlobStrikeView>,
    },
    Refused {
        request_reference: Uuid,
        event_id: Uuid,
        reason: ResourceErasureRefusalReason,
        detail: Option<String>,
    },
}

/// The request both doors take: the resource and the blocks whose history is scrubbed. Each
/// block must be a block of the resource, named once. `deny_unknown_fields`: the act's request
/// reference is minted by the service, so a caller that sends one is refused, not ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct BlockHistoryScrubRequestBody {
    pub resource: Uuid,
    pub blocks: Vec<Uuid>,
}

/// What the execute door's act did: a completion and a refusal are different answers, so the
/// response is a tagged enum. A refusal here is an operator-facing one (`charter_resource`,
/// `already_erased`); a caller who is not a system admin never reaches the act.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BlockHistoryScrubExecuteResponse {
    Completed {
        /// The server-minted reference the operator cites.
        request_reference: Uuid,
        event_id: Uuid,
        /// One line per named block, in the operator's order, then the ingest line when the
        /// scrub cancelled an in-flight ingest.
        targets: Vec<ErasureTargetOutcome>,
        /// True when the scrub cancelled an in-flight ingest.
        cancelled_ingest: bool,
    },
    Refused {
        request_reference: Uuid,
        event_id: Uuid,
        reason: ResourceErasureRefusalReason,
        detail: Option<String>,
        /// The blocks the refused act named, in the operator's order — the recorded refusal's
        /// `blocks`. Each is a block of the resource: the list is checked before a refusal is
        /// recorded.
        blocks: Vec<Uuid>,
    },
}

/// The request the field scrub's execute and survey doors take (field-grain scrub spec S1, S2): the
/// resource, the field by kind, the family's handle for `property`, and `clear`. No text: the field
/// is named by its kind and a family by the id of the first event still carrying its key text,
/// read from the family listing. `clear` is the operator's statement that today's value is part of
/// the leak; absent, today's value is kept and only the field's prior history is redacted.
/// `deny_unknown_fields`: the act's request reference is minted by the service, so a caller that
/// sends one is refused, not ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct FieldScrubRequestBody {
    pub resource: Uuid,
    pub field: ScrubFieldKind,
    /// The family's handle. Required for `property`, refused for every other field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<Uuid>,
    /// Clear today's value first, then redact every value the field held. Refused for
    /// `properties`: clear always names one field.
    #[serde(default)]
    pub clear: bool,
}

/// The family listing door's request: the resource and nothing else.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct FieldScrubFamiliesRequest {
    pub resource: Uuid,
}

/// One row of the family listing (field-grain scrub spec S1): the title, the origin URI, or one
/// resource-owned property family. Structure only, never a key or a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FieldScrubFamily {
    /// `title`, `origin_uri` or `property`.
    pub field: ScrubFieldKind,
    /// The family's handle, for `property`: the id of the first event still carrying its key text
    /// (the `doc_type` family's is the resource's `resource_created`). Absent for the title and
    /// origin URI, which need none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<EventId>,
    /// How many events carry the field.
    pub events: i32,
    /// Some row the field's events asserted is live. Always true for the title and origin URI.
    pub live: bool,
    /// The family has a `property_unset`, so its key text up to the latest one is scrubbable in
    /// keep mode.
    pub unset: bool,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    /// The profile behind the first event's emitter.
    pub first_by: Option<ProfileId>,
    /// The JSON type of the latest value the field carries; absent for a family only ever unset.
    pub value_type: Option<String>,
    /// The sensitivity sweep holds an open finding on a place of the field: a redactable path of one
    /// of its events, or the projection's title, origin URI or one of the family's rows. A flag
    /// confirms a leak; its absence never means the field is clean.
    #[serde(default)]
    pub flagged: bool,
    /// The same, on today's place only: the title or origin URI, or a live row. A flagged current
    /// value survives a scrub unless the request says `clear`.
    #[serde(default)]
    pub current_flagged: bool,
    /// The sweep has read all of the resource with every enabled detector. The same on every row.
    #[serde(default)]
    pub covered: bool,
}

/// The family listing door's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FieldScrubFamilies {
    pub resource: ResourceId,
    /// The title, then the origin URI, then each property family by its handle. Empty for an erased
    /// resource or a charter, which the act refuses before it reads a family.
    pub families: Vec<FieldScrubFamily>,
}

/// Paths of one event the scrub holds back, and why: `current` (today's value, kept),
/// `after_latest_unset` (key text after the family's latest unset) or `after_whole_facet_fold` (a
/// facet event after the latest whole-facet fold). Paths only, never values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FieldScrubHeldPaths {
    pub event: EventId,
    pub paths: Vec<String>,
    pub why: String,
}

/// An event clear mode appends before it redacts (field-grain scrub spec S2): a
/// `resource_updated` setting the placeholder title or origin URI, a `property_set` of `doc_type` to
/// the placeholder type, or a `property_unset` of the family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FieldScrubClearingEvent {
    pub event_type: String,
    /// The path of that event the clear writes.
    pub path: String,
}

/// The plan `resource_field_scrub_plan` computes (field-grain scrub spec S5): the act consumes the
/// same computation. Event ids, row ids and paths only. In keep mode an empty `redacted_fields`
/// means the field has nothing prior, and the act would answer 400.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FieldScrubPlan {
    /// The ledger paths the act would rewrite to their sentinels.
    pub redacted_fields: Vec<RedactedEventFields>,
    /// Today's value, kept (`why: current`).
    pub kept: Vec<FieldScrubHeldPaths>,
    /// Paths the act cannot reach, and why.
    pub unreachable: Vec<FieldScrubHeldPaths>,
    /// The folded `kb_properties` rows the act would rewrite to what replay projects.
    pub folded_rows: Vec<PropertyId>,
    /// The events clear mode would append first. Empty in keep mode.
    pub clears: Vec<FieldScrubClearingEvent>,
}

/// The read-only survey. Exactly one of `refusal` and `plan` is present: `refusal` when the act
/// would refuse, with `detail` for a charter; otherwise the `plan`. `families` is the listing,
/// absent exactly when the resource is a charter or erased. Nothing is recorded either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FieldScrubSurvey {
    pub resource: ResourceId,
    /// The refusal the act would record now.
    pub refusal: Option<FieldScrubRefusalReason>,
    /// The refusal's fixed evidence (a charter's map-grain task).
    pub detail: Option<String>,
    pub families: Option<Vec<FieldScrubFamily>>,
    pub plan: Option<FieldScrubPlan>,
}

/// What the execute door's act did: a completion and a refusal are different answers, so the
/// response is a tagged enum. A refusal here is an operator-facing one (`charter_resource`,
/// `already_erased`, `sentinel_collision`, `projection_disagrees`); a caller who is not a system
/// admin never reaches the act.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FieldScrubExecuteResponse {
    Completed {
        /// The server-minted reference the operator cites.
        request_reference: Uuid,
        event_id: Uuid,
        /// The field the act scrubbed, as its `resource_scrubbed` record names it.
        field: ScrubbedField,
        /// True when the act cleared today's value first.
        cleared: bool,
        /// The ledger paths the act rewrote to their sentinels.
        redacted_fields: Vec<RedactedEventFields>,
    },
    Refused {
        request_reference: Uuid,
        event_id: Uuid,
        reason: FieldScrubRefusalReason,
        detail: Option<String>,
        /// The field the refused act named. A family handle here is one of the resource's: the
        /// handle is checked before a refusal is recorded.
        field: ScrubbedField,
    },
}
