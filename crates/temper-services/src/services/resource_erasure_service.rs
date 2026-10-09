//! The resource-erasure act's service layer — execute + survey (resource erasure spec 2026-09-28,
//! build order 2b; D5, D8, D10). The principal act's service (`erasure_service`) is the template:
//! the same gate order, the same silent survey, the same wire-struct decoding of the act's jsonb.
//!
//! THE GATE IS `is_system_admin` AND NOTHING MORE (ruled 2026-09-30). "Operator + tenant" means
//! the instance: there is no tenant axis — `is_system_admin` (20260720000100) is the single
//! gating team's owner — and the act reaches a resource in any context (D5), so no per-context
//! check is added. Both functions take the sealed [`SystemAdmin`] proof, so the gate is the
//! signature and runs before any SQL touches the resource (authz-before-writes): a caller who is
//! not a system admin cannot reach them, and the surface that mints the proof rejects that caller
//! before dispatch with no ledger event (`handlers::resource_erasure`, ruled 2026-09-30).
//! Existence is disclosed only to an operator, as [`ApiError::NotFound`], never as raise text.
//!
//! SQL commits, it does not decide legality: the plan, the folds, the strikes and the redaction
//! body live in `resource_erasure_survey_plan` / `resource_erasure_execute` /
//! `resource_erasure_refuse` (migration 20260929040730). Two things diverge from the principal
//! door ON PURPOSE:
//!
//! * **The request reference is minted here**, one `Uuid::now_v7()` per act, never taken from a
//!   caller. It is the act's correlation id, and replay finds the act's span by it (D14): a
//!   reused reference would merge two acts' spans, so it is never reused and never batched. A
//!   refusal recorded after a raised execute carries the same reference — the aborted execute
//!   appended nothing, so the reference names exactly one attempt.
//! * **The act's raises are mapped**, through a closed classifier (`classify_act_failure`).
//!   Every raise in the act is a bare `RAISE EXCEPTION` (SQLSTATE P0001, no ERRCODE, HINT or
//!   DETAIL), so the classifier matches the message on stable prefixes and suffixes, pinned by
//!   unit tests over every literal and by a test-db witness against the live SQL. A refused
//!   state (charter, already erased) becomes a recorded refusal; a deadlock (`40P01`) or a
//!   raced edge fold retries a bounded number of times; nothing the SQL says reaches a response.
//!
//! The provider bytes of a released strike are deleted AFTER the act commits (a provider call
//! cannot join the transaction), once per released strike, and only when a store is configured.
//! NOTHING AFTER THE COMMIT CAN TURN THE ACT INTO A FAILURE: the strike labels are built from the
//! act's own returned `targets` with no further read, and a failed, skipped or timed-out release
//! is logged with the request reference and left to the byte-delete fence
//! (`erasure_fence_service`), which derives the same deletes from the `resource_erased` payload
//! and retries them with age alerting (derive-don't-remember). The HTTP doors call straight into
//! [`execute_resource_erasure`] / [`survey_resource_erasure`]; this module carries no HTTP types.

use std::collections::HashSet;
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::{
    BlobCoLinks, DeriverFingerprint, FingerprintMatch, OtherAuthorEdge, OtherAuthorEdgeProperty,
    ResourceErasurePlan, ResourceErasureSurvey,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{BlobId, EdgeId, EntityId, ProfileId, PropertyId, ResourceId};
use temper_core::types::workflow_job::{AnchorJobPayload, DispatchType, Persona};
use temper_substrate::blob_store::BlobStore;
use temper_substrate::payloads::{
    ErasureAct, ErasureTargetOutcome, RedactedEventFields, ResourceErasureRefusalReason,
};
use temper_substrate::writes::{release_blob_bytes, resolve_emitter};
use temper_workflow::operations::Surface;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};
use crate::services::erasure_fence_service::{
    classify_blob_outcome, content_hash_of_pathname, BlobOutcomeClass, BLOB_TARGET,
};
use crate::services::erasure_service::BlobStrikeOutcome;
use crate::services::workflow_job_service;

/// How many times one act re-runs after a retryable failure (a deadlock or a raced edge fold)
/// before it answers `Internal`. Two retries, three attempts: the byte-delete fence's bound for
/// the same deadlock class (`erasure_fence_service::drain`). The act is one statement and one
/// transaction, so a failed attempt commits nothing and each retry is a fresh statement against
/// the post-conflict state.
pub(super) const MAX_ACT_RETRIES: u32 = 2;

/// The one refusal detail today: a charter resource's erasure is map-grain, filed as its own task
/// (01a0e960-0ca2-7f42-b33e-1ed19b024e6b). Fixed text, because the refusal event is an admin
/// event that is never redactable: no caller-chosen text can reach it.
pub const MAP_GRAIN_ERASURE_DETAIL: &str =
    "map-grain erasure is task 01a0e960-0ca2-7f42-b33e-1ed19b024e6b";

/// How long the door waits on ONE post-commit provider delete before it stops waiting. Five
/// seconds: the act has already committed, and the byte-delete fence's drain is the backstop
/// that derives the same delete from the `resource_erased` payload and retries it with age
/// alerting, so a slow provider costs the operator a few seconds, never a gateway timeout on an
/// act that succeeded.
const POST_COMMIT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// The SQLSTATE Postgres raises when it resolves a deadlock by aborting one transaction.
pub(super) const DEADLOCK_DETECTED: &str = "40P01";

/// The SQLSTATE of a unique violation, retryable for one constraint only (below).
const UNIQUE_VIOLATION: &str = "23505";

/// The redaction rows' key. Two acts whose subjects share trail events, the two ends of one edge,
/// both derive the shared events' paths; the second to commit violates this key on the rows the
/// first just wrote. A retry re-derives against the rewritten ledger, finds those paths already at
/// their sentinels, and names them no more. A completion pass folds no edge, so it takes no edge
/// lock that would have serialized the two first (found by the code review, 2026-10-08).
const REDACTIONS_KEY: &str = "\"kb_event_field_redactions_pkey\"";

/// The SQLSTATE of a bare `RAISE EXCEPTION` (no ERRCODE), which every raise the classifier
/// matches by message is.
pub(super) const RAISE_EXCEPTION: &str = "P0001";

/// The prefix of every raise in `resource_erasure_execute` (migration 20260929040730).
const EXECUTE_RAISE_PREFIX: &str = "resource_erasure_execute: ";

/// The shape of a related-blob remainder entry the plan writes: `related blob <id>; hash …`.
const RELATED_BLOB_PREFIX: &str = "related blob ";

/// The shape of a deriver remainder entry the plan writes (D8):
/// `resource <id> holds a structural lead (…); never touched; discovery-bound`.
const DERIVER_TARGET: &str = "deriver";
const DERIVER_PREFIX: &str = "resource ";
const DERIVER_LEAD: &str = " holds a structural lead";

pub(super) const RESOURCE_NOT_FOUND: &str = "resource not found";

/// The closed vocabulary of a refusal's `detail`. A type, not a `String`, so free text can never
/// reach the never-redactable refusal event (the length bound the SQL lacks is unnecessary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceErasureRefusalDetail {
    /// A charter (cogmap telos) resource: map-grain erasure is its own act and task.
    MapGrainErasureTask,
}

impl ResourceErasureRefusalDetail {
    /// The fixed text recorded on the refusal event.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MapGrainErasureTask => MAP_GRAIN_ERASURE_DETAIL,
        }
    }
}

/// One execute request. The operator is not here (the [`SystemAdmin`] proof names it), and
/// neither is the request reference: the service mints it.
#[derive(Debug, Clone, Copy)]
pub struct ResourceErasureRequest<'a> {
    pub resource: ResourceId,
    /// The blobs the operator lists for striking (D8): each must be named in the survey's
    /// related-blob remainder, or the act refuses the whole request.
    pub also_strike_blobs: &'a [BlobId],
    /// Where the request came from; the act and any refusal are attributed through it.
    pub surface: Surface,
}

/// A completed resource erasure: ONE `resource_erased` event stands behind these values.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceErasureCompletion {
    /// The server-minted reference the act is correlated by — the operator cites it.
    pub request_reference: Uuid,
    pub event_id: Uuid,
    /// Every edge this act folded, each by its own `relationship_folded` event.
    pub folded_edges: Vec<EdgeId>,
    pub targets: Vec<ErasureTargetOutcome>,
    /// What the act names and does not touch, by design (D8).
    pub remainder: Vec<ErasureTargetOutcome>,
    /// The resource's own ledger paths the act rewrote to their sentinels (D3).
    pub redacted_fields: Vec<RedactedEventFields>,
    /// The ledger paths carrying the resource's content that the act cannot reach (D12).
    pub ledger_remainder: Vec<RedactedEventFields>,
    /// The operator-listed strikes, in the operator's order, each with its strike-time verdict.
    pub blob_strikes: Vec<BlobStrikeOutcome>,
}

/// A recorded refusal: one `resource_erasure_refused` event, nothing else mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceErasureRefusal {
    /// The server-minted reference the refusal event is correlated by.
    pub request_reference: Uuid,
    pub event_id: Uuid,
    pub reason: ResourceErasureRefusalReason,
    pub detail: Option<ResourceErasureRefusalDetail>,
}

/// What an execute call did.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceErasureOutcome {
    Completed(ResourceErasureCompletion),
    Refused(ResourceErasureRefusal),
}

/// The jsonb `resource_erasure_execute` returns.
#[derive(Debug, serde::Deserialize)]
struct ExecuteOutcomeWire {
    event_id: Uuid,
    edges: Vec<EdgeId>,
    targets: Vec<ErasureTargetOutcome>,
    remainder: Vec<ErasureTargetOutcome>,
    redacted_fields: Vec<RedactedEventFields>,
    ledger_remainder: Vec<RedactedEventFields>,
}

/// The jsonb `resource_erasure_survey` returns (its `resource` key is ignored: the caller named
/// it).
#[derive(Debug, serde::Deserialize)]
struct SurveyPlanWire {
    n_blocks: i64,
    n_revisions: i64,
    n_chunks: i64,
    n_artifacts: i64,
    n_edges: i64,
    edges: Vec<EdgeId>,
    targets: Vec<ErasureTargetOutcome>,
    already_erased: bool,
    charter_of: Option<Uuid>,
    ingest_state: String,
    fingerprint_available: bool,
    remainder: Vec<ErasureTargetOutcome>,
    redacted_fields: Vec<RedactedEventFields>,
    ledger_remainder: Vec<RedactedEventFields>,
}

/// Who is attempting which act, under which reference: everything a refusal records.
#[derive(Debug, Clone, Copy)]
pub(super) struct Attempt {
    pub(super) operator: ProfileId,
    pub(super) emitter: EntityId,
    pub(super) resource: ResourceId,
    pub(super) request_reference: Uuid,
}

/// How one run of the act ended, once the classifier has read any failure.
#[derive(Debug)]
enum ActVerdict {
    Completed(ExecuteOutcomeWire),
    Refused(
        ResourceErasureRefusalReason,
        Option<ResourceErasureRefusalDetail>,
    ),
}

/// The closed classification of a failed `resource_erasure_execute` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActFailure {
    Charter,
    AlreadyErased,
    NotFound,
    /// A listed blob the plan did not name; the id when the message carried a parseable one.
    BlobNotInRemainder(Option<Uuid>),
    /// A listed blob a previous act already struck.
    BlobAlreadyStruck(Option<Uuid>),
    /// A deadlock, or an edge folded between the plan and the fold loop.
    Retryable,
    Other,
}

/// Execute the resource-erasure act, as the operator `admin` names.
///
/// The [`SystemAdmin`] proof is the gate, and it ran where the proof was minted. The operator's
/// emitter resolves first (an unattributable authority act is worse than a failed one). An
/// unknown resource is [`ApiError::NotFound`] (existence is disclosed only to an operator). A
/// charter or an already-erased resource is a recorded refusal (the effect of a repeat erasure is
/// a no-op: no second `resource_erased` is minted). A listed blob the act refuses to strike is
/// [`ApiError::BadRequest`] naming that blob; the act rolled back whole, so nothing was struck.
/// A list naming one blob twice is a [`ApiError::BadRequest`] before the act runs.
pub async fn execute_resource_erasure(
    pool: &PgPool,
    store: Option<&dyn BlobStore>,
    admin: &SystemAdmin,
    request: ResourceErasureRequest<'_>,
) -> ApiResult<ResourceErasureOutcome> {
    execute_with_release_timeout(pool, store, admin, request, POST_COMMIT_RELEASE_TIMEOUT).await
}

/// [`execute_resource_erasure`] with the post-commit release bound as a parameter, so a witness
/// can shorten it.
async fn execute_with_release_timeout(
    pool: &PgPool,
    store: Option<&dyn BlobStore>,
    admin: &SystemAdmin,
    request: ResourceErasureRequest<'_>,
    release_timeout: Duration,
) -> ApiResult<ResourceErasureOutcome> {
    let operator = admin.actor();
    let emitter = resolve_emitter(pool, operator, request.surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let attempt = Attempt {
        operator,
        emitter,
        resource: request.resource,
        request_reference: Uuid::now_v7(),
    };

    // Operator-only from here: the body is validated, and existence may be disclosed, as an
    // error, not a ledger row.
    reject_duplicate_blobs(request.also_strike_blobs)?;
    if erased_state(pool, request.resource).await?.is_none() {
        return Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string()));
    }

    let wire = match run_act(pool, &attempt, request.also_strike_blobs).await? {
        ActVerdict::Completed(wire) => wire,
        ActVerdict::Refused(reason, detail) => {
            let refusal = refuse(pool, &attempt, ErasureAct::Erasure, &[], reason, detail).await?;
            return Ok(ResourceErasureOutcome::Refused(refusal));
        }
    };

    // COMMITTED. Nothing below may answer as a failure: the operator would retry and record an
    // `already_erased` refusal against an act that succeeded.
    let blob_strikes = label_strikes(
        attempt.request_reference,
        request.also_strike_blobs,
        &wire.targets,
    );
    if let Some(store) = store {
        release_struck_bytes(
            pool,
            store,
            attempt.request_reference,
            &wire.targets,
            release_timeout,
        )
        .await;
    }
    queue_region_settling(pool, attempt.resource, attempt.emitter).await;

    Ok(ResourceErasureOutcome::Completed(
        ResourceErasureCompletion {
            request_reference: attempt.request_reference,
            event_id: wire.event_id,
            folded_edges: wire.edges,
            targets: wire.targets,
            remainder: wire.remainder,
            redacted_fields: wire.redacted_fields,
            ledger_remainder: wire.ledger_remainder,
            blob_strikes,
        },
    ))
}

/// Queue a region settling for every anchor whose formation watermark the act nulled (D2 step 6):
/// the resource's home context, and each cogmap holding a LIVE region with the resource as a
/// member — the same two predicates as the act's step 6, read after commit (the act keeps the home
/// row and the member rows). The act has already recomputed those live centroids over the
/// survivors; the settling is what removes the husk from the regions' membership, re-derives their
/// readouts, and re-arms the context's telos snapshot the act nulled. A materialize already in
/// flight absorbs this job; `region_service::requeue_if_erased_members` follows it with another.
///
/// **Never fails the act**, as `DbBackend::queue_region_clocks` never fails a write: the act has
/// committed, and a failed enqueue leaves the regions to the next write that reaches the anchor.
async fn queue_region_settling(pool: &PgPool, resource: ResourceId, emitter: EntityId) {
    let anchors = match sqlx::query!(
        r#"SELECT 'kb_contexts' AS "anchor_table!", h.anchor_id AS "anchor_id!"
             FROM kb_resource_homes h
            WHERE h.resource_id = $1 AND h.anchor_table = 'kb_contexts'
           UNION
           SELECT 'kb_cogmaps', r.home_anchor_id
             FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources' AND mem.member_id = $1"#,
        resource.uuid(),
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(
                resource = %resource.uuid(),
                error = %e,
                "failed to read the erased resource's region anchors; regions are stale until the next write re-drives them"
            );
            return;
        }
    };
    for row in anchors {
        let Some(anchor) = HomeAnchor::from_parts(&row.anchor_table, row.anchor_id) else {
            tracing::warn!(
                anchor_table = %row.anchor_table,
                anchor = %row.anchor_id,
                "unknown anchor table for an erased resource's region; no settling queued"
            );
            continue;
        };
        let payload = AnchorJobPayload {
            emitter: emitter.uuid(),
        };
        if let Err(e) = workflow_job_service::enqueue_anchor(
            pool,
            anchor,
            Persona::Region.as_str(),
            DispatchType::Materialize.as_str(),
            payload,
        )
        .await
        {
            tracing::warn!(
                anchor = %anchor.uuid(),
                error = %e,
                "failed to queue region settling after erasure; regions are stale until the next write re-drives them"
            );
        }
    }
}

/// A list naming one blob twice is refused before the act touches the resource: the act would
/// strike it on its first mention and raise on its second as "struck by an earlier act", a false
/// account of the request. The id is formatted through `BlobId`'s own `Display` (the hyphenated
/// UUID).
fn reject_duplicate_blobs(blobs: &[BlobId]) -> ApiResult<()> {
    reject_duplicates(blobs, "blob", "struck")
}

/// A list naming one `noun` twice is a 400 naming the first repeated id, worded
/// "`noun` `id` is listed more than once; list each `noun` once; nothing was `undone`". Shared
/// by every act that takes an operator's list of ids.
pub(super) fn reject_duplicates<T>(items: &[T], noun: &str, undone: &str) -> ApiResult<()>
where
    T: Copy + Eq + std::hash::Hash + std::fmt::Display,
{
    let mut seen = HashSet::with_capacity(items.len());
    match items.iter().find(|i| !seen.insert(**i)) {
        Some(dup) => Err(ApiError::BadRequest(format!(
            "{noun} {dup} is listed more than once; list each {noun} once; nothing was {undone}"
        ))),
        None => Ok(()),
    }
}

/// `None` when no such resource exists; otherwise whether it is already erased.
pub(super) async fn erased_state(pool: &PgPool, resource: ResourceId) -> ApiResult<Option<bool>> {
    let erased = sqlx::query_scalar!(
        r#"SELECT erased_at IS NOT NULL AS "erased!" FROM kb_resources WHERE id = $1"#,
        resource.uuid(),
    )
    .fetch_optional(pool)
    .await?;
    Ok(erased)
}

/// Run the act, retrying a retryable failure up to [`MAX_ACT_RETRIES`] times, and map every
/// other failure through the classifier.
async fn run_act(
    pool: &PgPool,
    attempt: &Attempt,
    also_strike_blobs: &[BlobId],
) -> ApiResult<ActVerdict> {
    let blobs: Vec<Uuid> = also_strike_blobs.iter().map(|b| b.uuid()).collect();
    let mut retries = 0;
    loop {
        let result = sqlx::query_scalar!(
            r#"SELECT resource_erasure_execute($1, $2, $3, $4, $5)
                   AS "outcome: serde_json::Value""#,
            attempt.resource.uuid(),
            attempt.operator.uuid(),
            attempt.emitter.uuid(),
            attempt.request_reference,
            &blobs[..],
        )
        .fetch_one(pool)
        .await;
        let err = match result {
            Ok(raw) => return decode_completion(raw).map(ActVerdict::Completed),
            Err(err) => err,
        };
        match classify_act_error(&err) {
            ActFailure::Retryable if retries < MAX_ACT_RETRIES => retries += 1,
            failure => return verdict_for(failure, also_strike_blobs, err),
        }
    }
}

fn decode_completion(raw: Option<serde_json::Value>) -> ApiResult<ExecuteOutcomeWire> {
    let raw = raw.ok_or_else(|| {
        ApiError::Internal("resource_erasure_execute returned no row".to_string())
    })?;
    serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("resource erasure outcome shape: {e}")))
}

/// What a classified failure means for the caller. Never carries the raise text: a refusal is
/// recorded vocabulary, a blob refusal is a fixed message naming the operator's own blob id, and
/// anything unexpected is a scrubbed `Internal` (logged, not rendered).
fn verdict_for(
    failure: ActFailure,
    supplied: &[BlobId],
    err: sqlx::Error,
) -> ApiResult<ActVerdict> {
    match failure {
        ActFailure::Charter => Ok(ActVerdict::Refused(
            ResourceErasureRefusalReason::CharterResource,
            Some(ResourceErasureRefusalDetail::MapGrainErasureTask),
        )),
        ActFailure::AlreadyErased => Ok(ActVerdict::Refused(
            ResourceErasureRefusalReason::AlreadyErased,
            None,
        )),
        ActFailure::NotFound => Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string())),
        ActFailure::BlobNotInRemainder(blob) => Err(blob_refusal(
            blob,
            supplied,
            "is not a related blob of this resource (the survey's remainder names the blobs \
             that may be listed); nothing was struck",
        )),
        ActFailure::BlobAlreadyStruck(blob) => Err(blob_refusal(
            blob,
            supplied,
            "was struck by an earlier act and cannot be struck again; nothing was struck",
        )),
        ActFailure::Retryable | ActFailure::Other => Err(ApiError::internal_scrubbed(
            "resource erasure act failed",
            err,
        )),
    }
}

/// A 400 naming the blob the operator listed. The id comes from the raise only when it is one
/// of the operator's own; otherwise the message names no id.
fn blob_refusal(blob: Option<Uuid>, supplied: &[BlobId], reason: &str) -> ApiError {
    let supplied: Vec<Uuid> = supplied.iter().map(|s| s.uuid()).collect();
    listed_refusal("blob", blob, &supplied, reason)
}

/// A 400 naming the `noun` the operator listed: the id parsed from a raise is named only when
/// it is one of the operator's own (`supplied`); otherwise the message names no id. Shared by
/// every act that refuses an operator's listed id.
pub(super) fn listed_refusal(
    noun: &str,
    id: Option<Uuid>,
    supplied: &[Uuid],
    reason: &str,
) -> ApiError {
    match id.filter(|i| supplied.contains(i)) {
        Some(id) => ApiError::BadRequest(format!("{noun} {id} {reason}")),
        None => ApiError::BadRequest(format!("a listed {noun} {reason}")),
    }
}

/// Classify a failed act call. A non-database error (a lost connection, a decode failure) is
/// `Other`.
fn classify_act_error(err: &sqlx::Error) -> ActFailure {
    match err.as_database_error() {
        Some(db) => classify_act_failure(db.code().as_deref(), db.message()),
        None => ActFailure::Other,
    }
}

/// The pure classifier over a database error's SQLSTATE and message. The literals it matches
/// are the act's own raises (migration 20260929040730) and `blob_delete`'s already-struck raise
/// (20260906000010), all SQLSTATE `P0001`; each `%` in them is an id, so each arm matches a
/// stable prefix and suffix. A deadlock is matched by its SQLSTATE alone.
/// `p_resource is required` and `p_request_ref is required` are `Other`: the service always
/// supplies both, so either raise is a bug here, not a state of the resource.
fn classify_act_failure(code: Option<&str>, message: &str) -> ActFailure {
    if code == Some(DEADLOCK_DETECTED)
        || (code == Some(UNIQUE_VIOLATION) && message.contains(REDACTIONS_KEY))
    {
        return ActFailure::Retryable;
    }
    // Every message arm is a bare RAISE: the same text under any other SQLSTATE is not the act's.
    if code != Some(RAISE_EXCEPTION) {
        return ActFailure::Other;
    }
    if let Some(rest) = message.strip_prefix(EXECUTE_RAISE_PREFIX) {
        return classify_execute_raise(rest);
    }
    if let Some(id) = message
        .strip_prefix("blob_delete: blob ")
        .and_then(|rest| rest.split_once(" is already struck"))
        .map(|(id, _)| id)
    {
        return ActFailure::BlobAlreadyStruck(Uuid::parse_str(id).ok());
    }
    ActFailure::Other
}

/// The arms of `resource_erasure_execute`'s raises, after its prefix.
fn classify_execute_raise(rest: &str) -> ActFailure {
    if rest == "already erased" {
        ActFailure::AlreadyErased
    } else if rest.starts_with("charter resource") {
        ActFailure::Charter
    } else if let Some(id) = between(
        rest,
        "blob ",
        " is not in the survey's related-blob remainder; strike refused",
    ) {
        ActFailure::BlobNotInRemainder(Uuid::parse_str(id).ok())
    } else if between(rest, "edge ", " missing or already folded").is_some() {
        ActFailure::Retryable
    } else if between(rest, "remote source ", " gained a citer during the act").is_some()
        || between(rest, "remote source ", " lost its citers during the act").is_some()
    {
        // The plan read the source as exclusive; a write elsewhere cited it before step (9e)
        // locked it (20261015100020). The retry's plan names it shared.
        ActFailure::Retryable
    } else if between(rest, "resource ", " not found").is_some() {
        ActFailure::NotFound
    } else {
        ActFailure::Other
    }
}

pub(super) fn between<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    s.strip_prefix(prefix)?.strip_suffix(suffix)
}

/// Record an operator-facing refusal: ONE `resource_erasure_refused` event, nothing else mutated.
/// Attributed to the operator through the request's surface, correlated by the attempt's
/// reference. Both acts record their refusals here.
///
/// THE `act` RULE: the erasure records `p_act = NULL` and NULL blocks, so its refusal payloads
/// carry neither key and stay the shape every earlier erasure refusal has (an absent `act` reads
/// as [`ErasureAct::Erasure`]). The block history scrub records `p_act = 'block_history_scrub'`
/// and the blocks the operator named, each a block of the resource: the scrub's doors check
/// membership before they record a refusal. An empty `blocks` is passed as NULL; the SQL
/// (`resource_erasure_refuse`, migration 20261003000210) accepts blocks only beside the scrub's
/// act.
pub(super) async fn refuse(
    pool: &PgPool,
    attempt: &Attempt,
    act: ErasureAct,
    blocks: &[Uuid],
    reason: ResourceErasureRefusalReason,
    detail: Option<ResourceErasureRefusalDetail>,
) -> ApiResult<ResourceErasureRefusal> {
    let reason_str = serde_json::to_value(reason)
        .expect("a refusal reason always serializes")
        .as_str()
        .expect("a refusal reason serializes to a string")
        .to_string();
    let recorded_act: Option<&str> = match act {
        ErasureAct::Erasure => None,
        ErasureAct::BlockHistoryScrub => Some("block_history_scrub"),
        ErasureAct::FieldScrub => Some("field_scrub"),
    };
    let recorded_blocks: Option<&[Uuid]> = (!blocks.is_empty()).then_some(blocks);

    let event_id: Uuid = sqlx::query_scalar!(
        r#"SELECT resource_erasure_refuse($1, $2, $3, $4, $5, $6, $7, $8) AS "event: Uuid""#,
        attempt.resource.uuid(),
        attempt.operator.uuid(),
        attempt.emitter.uuid(),
        attempt.request_reference,
        reason_str,
        detail.map(ResourceErasureRefusalDetail::as_str),
        recorded_act,
        recorded_blocks,
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| ApiError::Internal("resource_erasure_refuse returned no row".to_string()))?;

    Ok(ResourceErasureRefusal {
        request_reference: attempt.request_reference,
        event_id,
        reason,
        detail,
    })
}

/// The operator's strikes, in the operator's order, each labelled with its verdict. Built from
/// the act's returned `targets` alone, with no read after the commit. The act appends one
/// `kb_blobs` target per listed blob, in list order (the strike loop of
/// `resource_erasure_execute`, 20260929040730; the plan's own `kb_blobs` entries go to the
/// remainder, never to `targets`), so the pairing is positional, and each verdict is read
/// through the fence's own parser. A count mismatch or an unrecognized verdict is LOGGED with
/// the request reference, never returned: the act has committed, its record's `targets` still
/// carry every verdict, and the fence alerts on one it cannot parse.
fn label_strikes(
    request_reference: Uuid,
    listed: &[BlobId],
    targets: &[ErasureTargetOutcome],
) -> Vec<BlobStrikeOutcome> {
    let verdicts: Vec<&ErasureTargetOutcome> =
        targets.iter().filter(|t| t.target == BLOB_TARGET).collect();
    if verdicts.len() != listed.len() {
        tracing::error!(
            request_reference = %request_reference,
            listed = listed.len(),
            verdicts = verdicts.len(),
            "resource erasure committed, but its strike verdicts do not pair with the listed \
             blobs; no strike is labelled, and the record's targets carry every verdict"
        );
        return Vec::new();
    }
    listed
        .iter()
        .zip(verdicts)
        .map(|(blob, verdict)| {
            let released = match classify_blob_outcome(&verdict.outcome) {
                BlobOutcomeClass::Released(_) => true,
                BlobOutcomeClass::Known => false,
                BlobOutcomeClass::Unrecognized => {
                    tracing::error!(
                        request_reference = %request_reference,
                        blob = %blob,
                        "resource erasure committed, but a strike verdict has no known shape; \
                         labelled unreleased, and the fence counts it as unparseable"
                    );
                    false
                }
            };
            BlobStrikeOutcome {
                blob_id: blob.uuid(),
                released,
            }
        })
        .collect()
}

/// The post-commit byte release, once per released strike in the act's `targets` — the
/// `blob_service` delete door's shape: `release_blob_bytes` re-derives released-ness under the
/// hash lock and holds it across the provider delete. The hash is derived FROM the pathname, as
/// the fence derives it, so the lock and the deleted pathname have one source. Each release is
/// bounded by `timeout`. A skip, a failure or a timeout is logged with the request reference,
/// never a door failure: the fence seeds the same pathname from the `resource_erased` payload
/// and retries it with age alerting.
async fn release_struck_bytes(
    pool: &PgPool,
    store: &dyn BlobStore,
    request_reference: Uuid,
    targets: &[ErasureTargetOutcome],
    timeout: Duration,
) {
    for target in targets.iter().filter(|t| t.target == BLOB_TARGET) {
        let BlobOutcomeClass::Released(pathname) = classify_blob_outcome(&target.outcome) else {
            continue;
        };
        let content_hash = content_hash_of_pathname(&pathname);
        let release = release_blob_bytes(pool, content_hash, &pathname, store);
        match tokio::time::timeout(timeout, release).await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => tracing::info!(
                request_reference = %request_reference,
                pathname = %pathname,
                "post-commit release skipped: a live row re-holds the hash — the fence \
                 resolves its seeded row re-occupied"
            ),
            Ok(Err(e)) => tracing::warn!(
                request_reference = %request_reference,
                pathname = %pathname,
                error = format!("{e:#}"),
                "post-commit provider delete failed — the byte-delete fence retries with \
                 age alerting"
            ),
            Err(_) => tracing::warn!(
                request_reference = %request_reference,
                pathname = %pathname,
                timeout = ?timeout,
                "post-commit provider delete timed out — the byte-delete fence retries with \
                 age alerting"
            ),
        }
    }
}

/// The read-only survey: what [`execute_resource_erasure`] would do if it ran now, rendered
/// from the act's own plan (`resource_erasure_survey_plan`, D10), plus display-only annotations.
///
/// The [`SystemAdmin`] proof is the gate, as for the act; the survey records nothing (a survey
/// attempt is not an erasure request). An unknown resource is `NotFound`, and an already-erased
/// one short-circuits to a minimal survey with no plan: the plan still counts rows on a husk,
/// which would misstate what an act could reach. What an act CAN still reach on a husk is its
/// completion pass (D12), named by `completion_fields` from the derivation the act runs.
pub async fn survey_resource_erasure(
    pool: &PgPool,
    _admin: &SystemAdmin,
    resource: ResourceId,
) -> ApiResult<ResourceErasureSurvey> {
    match erased_state(pool, resource).await? {
        None => return Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string())),
        Some(true) => {
            let raw = sqlx::query_scalar!(
                r#"SELECT resource_erasure_completion_fields($1) AS "fields!: serde_json::Value""#,
                resource.uuid(),
            )
            .fetch_one(pool)
            .await?;
            let completion_fields = serde_json::from_value(raw).map_err(|e| {
                ApiError::Internal(format!("resource erasure completion fields shape: {e}"))
            })?;
            let derivers = deriver_ids(&first_record_remainder(pool, resource).await?)?;
            let deriver_fingerprints = deriver_fingerprints(pool, resource, &derivers).await?;
            return Ok(ResourceErasureSurvey {
                resource,
                already_erased: true,
                plan: None,
                completion_fields,
                deriver_fingerprints,
            });
        }
        Some(false) => {}
    }

    let raw = sqlx::query_scalar!(
        r#"SELECT resource_erasure_survey($1) AS "survey: serde_json::Value""#,
        resource.uuid(),
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| ApiError::Internal("resource_erasure_survey returned no row".to_string()))?;
    let wire: SurveyPlanWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("resource erasure survey shape: {e}")))?;

    let other_author_edges = other_author_edges(pool, resource).await?;
    let other_author_edge_properties = other_author_edge_properties(pool, resource).await?;
    let blob_co_links = blob_co_links(pool, resource, &wire.remainder).await?;
    let deriver_fingerprints =
        deriver_fingerprints(pool, resource, &deriver_ids(&wire.remainder)?).await?;

    Ok(ResourceErasureSurvey {
        resource,
        already_erased: wire.already_erased,
        plan: Some(ResourceErasurePlan {
            n_blocks: wire.n_blocks,
            n_revisions: wire.n_revisions,
            n_chunks: wire.n_chunks,
            n_artifacts: wire.n_artifacts,
            n_edges: wire.n_edges,
            edges: wire.edges,
            targets: wire.targets,
            charter_of: wire.charter_of,
            ingest_state: wire.ingest_state,
            fingerprint_available: wire.fingerprint_available,
            remainder: wire.remainder,
            deriver_fingerprints,
            redacted_fields: wire.redacted_fields,
            ledger_remainder: wire.ledger_remainder,
            other_author_edges,
            other_author_edge_properties,
            blob_co_links,
        }),
        completion_fields: Vec::new(),
        deriver_fingerprints: Vec::new(),
    })
}

/// Every edge touching the resource — live or already folded, the reach of the act's label and
/// property steps (9c, 9d) — whose asserting principal is not the resource's owner.
///
/// Authorship is not a column on `kb_edges`: it is the emitter of the edge's asserting event
/// (`asserted_by_event_id` → `kb_events.emitter_entity_id`), the actor `element_trail_edge`
/// reports. The comparison is PROFILE to PROFILE: the owner is
/// `kb_resource_homes.owner_profile_id` and the emitting entity is resolved to its `profile_id`,
/// because an entity is one surface of a principal (`<handle>@web`, `<handle>@cli`), and the
/// owner's own edge asserted from another surface is not another principal's text.
async fn other_author_edges(
    pool: &PgPool,
    resource: ResourceId,
) -> ApiResult<Vec<OtherAuthorEdge>> {
    let rows = sqlx::query!(
        r#"
        SELECT e.id          AS "edge_id!: Uuid",
               en.profile_id AS "author!: Uuid",
               e.is_folded   AS "folded!"
          FROM kb_edges e
          JOIN kb_events ev ON ev.id = e.asserted_by_event_id
          JOIN kb_entities en ON en.id = ev.emitter_entity_id
         WHERE ((e.source_table = 'kb_resources' AND e.source_id = $1)
             OR (e.target_table = 'kb_resources' AND e.target_id = $1))
           AND en.profile_id IS DISTINCT FROM
               (SELECT h.owner_profile_id FROM kb_resource_homes h WHERE h.resource_id = $1)
         ORDER BY e.id
        "#,
        resource.uuid(),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OtherAuthorEdge {
            edge_id: EdgeId::from(r.edge_id),
            author: ProfileId::from(r.author),
            folded: r.folded,
        })
        .collect())
}

/// Every property row owned by an edge touching the resource (live or folded, every row step 9d
/// sentinels) whose asserting principal is not the resource's owner — the same derivation and
/// the same profile-to-profile comparison as [`other_author_edges`], over the row's own
/// `asserted_by_event_id`.
async fn other_author_edge_properties(
    pool: &PgPool,
    resource: ResourceId,
) -> ApiResult<Vec<OtherAuthorEdgeProperty>> {
    let rows = sqlx::query!(
        r#"
        SELECT p.id          AS "property_id!: Uuid",
               e.id          AS "edge_id!: Uuid",
               en.profile_id AS "author!: Uuid",
               p.is_folded   AS "folded!"
          FROM kb_properties p
          JOIN kb_edges e ON p.owner_table = 'kb_edges' AND e.id = p.owner_id
          JOIN kb_events ev ON ev.id = p.asserted_by_event_id
          JOIN kb_entities en ON en.id = ev.emitter_entity_id
         WHERE ((e.source_table = 'kb_resources' AND e.source_id = $1)
             OR (e.target_table = 'kb_resources' AND e.target_id = $1))
           AND en.profile_id IS DISTINCT FROM
               (SELECT h.owner_profile_id FROM kb_resource_homes h WHERE h.resource_id = $1)
         ORDER BY p.id
        "#,
        resource.uuid(),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OtherAuthorEdgeProperty {
            property_id: PropertyId::from(r.property_id),
            edge_id: EdgeId::from(r.edge_id),
            author: ProfileId::from(r.author),
            folded: r.folded,
        })
        .collect())
}

/// For each related blob the plan's remainder names, the other resources holding a live edge
/// to it. The blobs are the plan's own (parsed from its remainder), never re-derived.
async fn blob_co_links(
    pool: &PgPool,
    resource: ResourceId,
    remainder: &[ErasureTargetOutcome],
) -> ApiResult<Vec<BlobCoLinks>> {
    let blobs = related_blob_ids(remainder)?;
    if blobs.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"
        SELECT CASE WHEN e.source_table = 'kb_blobs' THEN e.source_id ELSE e.target_id END
                   AS "blob_id!: Uuid",
               CASE WHEN e.source_table = 'kb_blobs' THEN e.target_id ELSE e.source_id END
                   AS "holder!: Uuid"
          FROM kb_edges e
         WHERE NOT e.is_folded
           AND ((e.source_table = 'kb_blobs' AND e.source_id = ANY($1)
                 AND e.target_table = 'kb_resources' AND e.target_id <> $2)
             OR (e.target_table = 'kb_blobs' AND e.target_id = ANY($1)
                 AND e.source_table = 'kb_resources' AND e.source_id <> $2))
         ORDER BY 1, 2
        "#,
        &blobs[..],
        resource.uuid(),
    )
    .fetch_all(pool)
    .await?;

    Ok(blobs
        .into_iter()
        .map(|blob| {
            let mut holders: Vec<ResourceId> = rows
                .iter()
                .filter(|r| r.blob_id == blob)
                .map(|r| ResourceId::from(r.holder))
                .collect();
            holders.dedup();
            BlobCoLinks {
                blob_id: BlobId::from(blob),
                holders,
            }
        })
        .collect())
}

/// Whether the sensitivity sweep confirms each deriver quotes one of the resource's detected values
/// (D10), in `derivers`' order. The derivers are the plan's own, never re-derived; the answer is
/// read through `resource_erasure_deriver_fingerprints`, since no Rust names the sweep's schema.
async fn deriver_fingerprints(
    pool: &PgPool,
    resource: ResourceId,
    derivers: &[Uuid],
) -> ApiResult<Vec<DeriverFingerprint>> {
    if derivers.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"SELECT deriver_id AS "deriver_id!: Uuid", state AS "state!"
             FROM resource_erasure_deriver_fingerprints($1, $2)"#,
        resource.uuid(),
        derivers,
    )
    .fetch_all(pool)
    .await?;
    in_deriver_order(
        resource,
        derivers,
        rows.into_iter().map(|r| (r.deriver_id, r.state)).collect(),
    )
}

/// The SQL's `(deriver, state)` rows in `derivers`' order. Only the resource itself goes
/// unanswered, by design; any other deriver missing, or a state outside the vocabulary, is drift
/// between the function and this caller, and fails loud.
fn in_deriver_order(
    resource: ResourceId,
    derivers: &[Uuid],
    rows: Vec<(Uuid, String)>,
) -> ApiResult<Vec<DeriverFingerprint>> {
    derivers
        .iter()
        .filter(|d| **d != resource.uuid())
        .map(|d| {
            let (_, state) = rows.iter().find(|(id, _)| id == d).ok_or_else(|| {
                ApiError::Internal(format!("the sweep did not answer for deriver {d}"))
            })?;
            let fingerprint_match = match state.as_str() {
                "yes" => FingerprintMatch::Yes,
                "no" => FingerprintMatch::No,
                "unscanned" => FingerprintMatch::Unscanned,
                "expired" => FingerprintMatch::Expired,
                other => {
                    return Err(ApiError::Internal(format!(
                        "the deriver fingerprint state {other:?} is not in the vocabulary"
                    )))
                }
            };
            Ok(DeriverFingerprint {
                deriver: ResourceId::from(*d),
                fingerprint_match,
            })
        })
        .collect()
}

/// The remainder the resource's first `resource_erased` record named. A completion pass's record
/// carries none (D12), so the first is the act that named the derivers. Every husk has one: a
/// husk without is drift, and fails loud.
async fn first_record_remainder(
    pool: &PgPool,
    resource: ResourceId,
) -> ApiResult<Vec<ErasureTargetOutcome>> {
    let raw = sqlx::query_scalar!(
        r#"SELECT coalesce(e.payload -> 'remainder', '[]'::jsonb) AS "remainder!: serde_json::Value"
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
            WHERE t.name = 'resource_erased' AND e.payload ->> 'subject_id' = $1
            ORDER BY e.id
            LIMIT 1"#,
        resource.uuid().to_string(),
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ApiError::Internal(format!(
            "the erased resource {} has no resource_erased record",
            resource.uuid()
        ))
    })?;
    serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("resource erasure record remainder shape: {e}")))
}

/// The resource ids of the plan's deriver remainder entries (`resource <id> holds a structural
/// lead …`), each once, in order. An entry in any other shape is drift in the plan's template,
/// and fails loud.
fn deriver_ids(remainder: &[ErasureTargetOutcome]) -> ApiResult<Vec<Uuid>> {
    let mut ids = Vec::new();
    for r in remainder.iter().filter(|r| r.target == DERIVER_TARGET) {
        let id = r
            .outcome
            .strip_prefix(DERIVER_PREFIX)
            .and_then(|rest| rest.split_once(DERIVER_LEAD))
            .and_then(|(id, _)| Uuid::parse_str(id).ok())
            .ok_or_else(|| {
                ApiError::Internal(
                    "the survey's remainder names a deriver in an unrecognized shape".to_string(),
                )
            })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// The blob ids of the plan's related-blob remainder entries (`related blob <id>; hash …`). An
/// entry in any other shape is drift in the plan's template, and fails loud.
fn related_blob_ids(remainder: &[ErasureTargetOutcome]) -> ApiResult<Vec<Uuid>> {
    remainder
        .iter()
        .filter(|r| r.target == BLOB_TARGET)
        .map(|r| {
            r.outcome
                .strip_prefix(RELATED_BLOB_PREFIX)
                .and_then(|rest| rest.split_once(';'))
                .and_then(|(id, _)| Uuid::parse_str(id).ok())
                .ok_or_else(|| {
                    ApiError::Internal(
                        "the survey's remainder names a blob in an unrecognized shape".to_string(),
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod classifier_tests {
    //! The classifier over every literal the act raises (migration 20260929040730) and
    //! `blob_delete`'s already-struck raise (20260906000010), `%` substituted with an id. Each
    //! FAILS IF the classifier's arm for that literal drifts.
    use super::*;

    const P0001: Option<&str> = Some("P0001");

    fn id() -> Uuid {
        Uuid::parse_str("01a0e9e7-491d-7700-8f58-99d0b068e059").expect("a literal uuid")
    }

    #[test]
    fn the_required_argument_raises_are_other() {
        assert_eq!(
            classify_act_failure(P0001, "resource_erasure_execute: p_resource is required"),
            ActFailure::Other
        );
        assert_eq!(
            classify_act_failure(P0001, "resource_erasure_execute: p_request_ref is required"),
            ActFailure::Other
        );
    }

    #[test]
    fn resource_not_found_is_not_found() {
        let msg = format!("resource_erasure_execute: resource {} not found", id());
        assert_eq!(classify_act_failure(P0001, &msg), ActFailure::NotFound);
    }

    #[test]
    fn the_charter_raise_is_charter() {
        assert_eq!(
            classify_act_failure(
                P0001,
                "resource_erasure_execute: charter resource (map-grain erasure is filed task \
                 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)"
            ),
            ActFailure::Charter
        );
    }

    #[test]
    fn already_erased_is_already_erased() {
        assert_eq!(
            classify_act_failure(P0001, "resource_erasure_execute: already erased"),
            ActFailure::AlreadyErased
        );
    }

    #[test]
    fn a_blob_outside_the_remainder_names_its_id() {
        let msg = format!(
            "resource_erasure_execute: blob {} is not in the survey's related-blob remainder; \
             strike refused",
            id()
        );
        assert_eq!(
            classify_act_failure(P0001, &msg),
            ActFailure::BlobNotInRemainder(Some(id()))
        );
    }

    #[test]
    fn a_raced_edge_fold_is_retryable() {
        let msg = format!(
            "resource_erasure_execute: edge {} missing or already folded",
            id()
        );
        assert_eq!(classify_act_failure(P0001, &msg), ActFailure::Retryable);
    }

    // The plan read a remote source as exclusive and a write elsewhere cited it before step (9e)
    // locked it (20261015100020): retried, so the next plan names it shared.
    #[test]
    fn a_remote_source_that_gained_a_citer_is_retryable() {
        let msg = format!(
            "resource_erasure_execute: remote source {} gained a citer during the act",
            id()
        );
        assert_eq!(classify_act_failure(P0001, &msg), ActFailure::Retryable);
        let msg = format!(
            "resource_erasure_execute: remote source {} lost its citers during the act",
            id()
        );
        assert_eq!(classify_act_failure(P0001, &msg), ActFailure::Retryable);
    }

    #[test]
    fn an_already_struck_blob_names_its_id() {
        let msg = format!(
            "blob_delete: blob {} is already struck — the ledger carries its emptying",
            id()
        );
        assert_eq!(
            classify_act_failure(P0001, &msg),
            ActFailure::BlobAlreadyStruck(Some(id()))
        );
    }

    #[test]
    fn a_deadlock_is_retryable_whatever_its_message() {
        assert_eq!(
            classify_act_failure(Some("40P01"), "deadlock detected"),
            ActFailure::Retryable
        );
    }

    // FAILS IF a message arm matches the act's text under a SQLSTATE that is not a bare RAISE.
    #[test]
    fn the_right_message_under_the_wrong_code_is_other() {
        let msg = format!(
            "resource_erasure_execute: edge {} missing or already folded",
            id()
        );
        assert_eq!(classify_act_failure(P0001, &msg), ActFailure::Retryable);
        assert_eq!(classify_act_failure(Some("XX000"), &msg), ActFailure::Other);
        assert_eq!(classify_act_failure(None, &msg), ActFailure::Other);
        assert_eq!(
            classify_act_failure(Some("23505"), "resource_erasure_execute: already erased"),
            ActFailure::Other
        );
    }

    // FAILS IF two acts racing over a shared edge's events answer 500 rather than retry, or if
    // any other unique violation is retried.
    #[test]
    fn a_collision_on_the_redaction_rows_is_retryable_and_no_other_unique_violation_is() {
        assert_eq!(
            classify_act_failure(
                Some("23505"),
                "duplicate key value violates unique constraint \"kb_event_field_redactions_pkey\""
            ),
            ActFailure::Retryable
        );
        assert_eq!(
            classify_act_failure(
                Some("23505"),
                "duplicate key value violates unique constraint \"kb_block_provenance_pkey\""
            ),
            ActFailure::Other
        );
    }

    #[test]
    fn an_unrelated_raise_is_other() {
        let msg = format!("resource {} is erased; writes are refused", id());
        assert_eq!(classify_act_failure(P0001, &msg), ActFailure::Other);
        assert_eq!(
            classify_act_failure(P0001, "resource_erasure_execute: something new"),
            ActFailure::Other
        );
    }

    fn blob_target(outcome: &str) -> ErasureTargetOutcome {
        ErasureTargetOutcome {
            target: BLOB_TARGET.to_string(),
            outcome: outcome.to_string(),
        }
    }

    // FAILS IF the strike labels stop pairing the listed blobs with the act's `kb_blobs` targets
    // in list order, or a verdict's class is misread.
    #[test]
    fn strikes_pair_positionally_with_the_acts_blob_targets() {
        let (a, b) = (BlobId::from(Uuid::now_v7()), BlobId::from(Uuid::now_v7()));
        let targets = vec![
            ErasureTargetOutcome {
                target: "kb_resources".to_string(),
                outcome: "husk".to_string(),
            },
            blob_target("erased; released=false; pathname=ab/abc"),
            blob_target("erased; released=true; pathname=cd/cde"),
        ];
        assert_eq!(
            label_strikes(Uuid::now_v7(), &[a, b], &targets),
            vec![
                BlobStrikeOutcome {
                    blob_id: Uuid::from(a),
                    released: false,
                },
                BlobStrikeOutcome {
                    blob_id: Uuid::from(b),
                    released: true,
                },
            ]
        );
    }

    // FAILS IF a strike record that does not pair with the list (a count mismatch, or a verdict
    // in no known shape) panics or is anything but a label set — the act has committed, and
    // `label_strikes` has no error to return by construction.
    #[test]
    fn a_strike_record_that_does_not_pair_is_logged_not_failed() {
        let a = BlobId::from(Uuid::now_v7());
        let two = vec![
            blob_target("erased; released=true; pathname=ab/abc"),
            blob_target("erased; released=true; pathname=cd/cde"),
        ];
        assert!(label_strikes(Uuid::now_v7(), &[a], &two).is_empty());
        assert!(label_strikes(Uuid::now_v7(), &[a], &[]).is_empty());
        assert_eq!(
            label_strikes(Uuid::now_v7(), &[a], &[blob_target("struck somehow")]),
            vec![BlobStrikeOutcome {
                blob_id: Uuid::from(a),
                released: false,
            }]
        );
    }

    // FAILS IF a list naming one blob twice passes, or the 400 does not name the blob.
    #[test]
    fn a_list_naming_a_blob_twice_is_a_400_naming_it() {
        let (a, b) = (BlobId::from(Uuid::now_v7()), BlobId::from(Uuid::now_v7()));
        assert!(reject_duplicate_blobs(&[a, b]).is_ok());
        assert!(reject_duplicate_blobs(&[]).is_ok());
        match reject_duplicate_blobs(&[a, b, a]) {
            Err(ApiError::BadRequest(msg)) => {
                assert!(msg.contains(&a.to_string()), "{msg}");
                assert!(!msg.contains(&b.to_string()), "{msg}");
            }
            other => panic!("a duplicate is a 400, got {other:?}"),
        }
    }

    // FAILS IF the charter detail stops naming the map-grain task, or starts carrying the raise.
    #[test]
    fn the_charter_detail_names_the_map_grain_task_and_not_the_raise() {
        let detail = ResourceErasureRefusalDetail::MapGrainErasureTask.as_str();
        assert!(detail.contains("01a0e960-0ca2-7f42-b33e-1ed19b024e6b"));
        assert!(!detail.contains(EXECUTE_RAISE_PREFIX));
    }

    // FAILS IF the remainder parse loses the plan's `related blob <id>; hash …` shape, or stops
    // failing loud on a drifted one.
    #[test]
    fn related_blob_ids_parse_the_plans_remainder_shape() {
        let remainder = vec![
            ErasureTargetOutcome {
                target: "kb_blobs".to_string(),
                outcome: format!(
                    "related blob {}; hash abc; live; struck only when the operator lists it",
                    id()
                ),
            },
            ErasureTargetOutcome {
                target: "deriver".to_string(),
                outcome: "resource x holds a structural lead".to_string(),
            },
        ];
        assert_eq!(related_blob_ids(&remainder).expect("parses"), vec![id()]);

        let drifted = vec![ErasureTargetOutcome {
            target: "kb_blobs".to_string(),
            outcome: format!("blob {} related", id()),
        }];
        assert!(related_blob_ids(&drifted).is_err());
    }

    // FAILS IF the deriver parse loses the plan's `resource <id> holds a structural lead (…)`
    // shape (20261009100000, the deriver loop), answers a deriver twice, or stops failing loud on
    // a drifted entry.
    #[test]
    fn deriver_ids_parse_the_plans_remainder_shape() {
        let lead = |kind: &str| ErasureTargetOutcome {
            target: "deriver".to_string(),
            outcome: format!(
                "resource {} holds a structural lead ({kind}); never touched; discovery-bound",
                id()
            ),
        };
        let remainder = vec![
            lead("derived_from edge"),
            ErasureTargetOutcome {
                target: "kb_blobs".to_string(),
                outcome: format!("related blob {}; hash abc; already struck", id()),
            },
            lead("provenance citation"),
        ];
        assert_eq!(deriver_ids(&remainder).expect("parses"), vec![id()]);

        let drifted = vec![ErasureTargetOutcome {
            target: "deriver".to_string(),
            outcome: format!("deriver {} named", id()),
        }];
        assert!(deriver_ids(&drifted).is_err());
    }

    /// FAILS IF a deriver the sweep did not answer is dropped rather than refused, if the resource
    /// itself is not the one deriver allowed to go unanswered, or if the answer leaves plan order.
    #[test]
    fn every_deriver_but_the_resource_is_answered_in_plan_order() {
        let r = Uuid::now_v7();
        let (d1, d2) = (Uuid::now_v7(), Uuid::now_v7());
        let rows = vec![(d2, "no".to_string()), (d1, "yes".to_string())];
        let answered =
            in_deriver_order(ResourceId::from(r), &[d1, r, d2], rows.clone()).expect("answers");
        assert_eq!(
            answered
                .iter()
                .map(|f| (f.deriver.uuid(), f.fingerprint_match))
                .collect::<Vec<_>>(),
            vec![(d1, FingerprintMatch::Yes), (d2, FingerprintMatch::No)]
        );

        let unanswered = Uuid::now_v7();
        assert!(in_deriver_order(ResourceId::from(r), &[d1, unanswered], rows.clone()).is_err());
        let drifted = vec![(d1, "maybe".to_string())];
        assert!(in_deriver_order(ResourceId::from(r), &[d1], drifted).is_err());
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    //! Service witnesses. Every test runs on a fresh database migrated by
    //! `temper_substrate::MIGRATOR` — no `reset_schema` — so every event type a migration
    //! registered is present (the substrate suite's re-registration trap does not apply).
    use bytes::Bytes;
    use sha2::Digest as _;
    use sqlx::PgPool;
    use uuid::Uuid;

    use temper_core::types::property_owner::PropertyOwner;
    use temper_substrate::affinity::EdgeKind;
    use temper_substrate::blob_store::{
        blob_pathname, BlobHead, ByteStream, InMemoryBlobStore, PutReceipt,
    };
    use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
    use temper_substrate::ids::ContextId;
    use temper_substrate::payloads::{AnchorRef, EdgePolarity};
    use temper_substrate::scenario::bootseed;
    use temper_substrate::writes::{
        self, AssertParams, CommitBlobParams, CreateMode, CreateParams,
    };

    use super::*;
    use crate::services::erasure_fence_service;
    use crate::test_support;

    /// The raise literals' distinctive parts: no response or error may carry any of them.
    const RAISE_FRAGMENTS: &[&str] = &[
        "resource_erasure_execute:",
        "blob_delete:",
        "related-blob remainder; strike refused",
        "missing or already folded",
        "the ledger carries its emptying",
        "map-grain erasure is filed task",
        "already erased",
    ];

    /// How a [`BadDeleteStore`]'s provider delete misbehaves.
    #[derive(Debug, Clone, Copy)]
    enum BadDelete {
        Fails,
        Hangs,
    }

    /// A provider whose delete errors or never returns, over a working in-memory store — the
    /// post-commit release's two bad days. Everything but `delete` passes through.
    #[derive(Debug)]
    struct BadDeleteStore {
        inner: InMemoryBlobStore,
        mode: BadDelete,
    }

    #[async_trait::async_trait]
    impl BlobStore for BadDeleteStore {
        async fn exists(&self, pathname: &str) -> anyhow::Result<bool> {
            self.inner.exists(pathname).await
        }
        async fn put(
            &self,
            pathname: &str,
            content_type: &str,
            body: Bytes,
            cache_control_max_age: u32,
        ) -> anyhow::Result<PutReceipt> {
            self.inner
                .put(pathname, content_type, body, cache_control_max_age)
                .await
        }
        async fn get(&self, pathname: &str, consistent: bool) -> anyhow::Result<ByteStream> {
            self.inner.get(pathname, consistent).await
        }
        async fn head(&self, pathname: &str) -> anyhow::Result<Option<BlobHead>> {
            self.inner.head(pathname).await
        }
        async fn delete(&self, _pathnames: &[&str]) -> anyhow::Result<()> {
            match self.mode {
                BadDelete::Fails => anyhow::bail!("provider unavailable (the witness's outage)"),
                BadDelete::Hangs => std::future::pending::<anyhow::Result<()>>().await,
            }
        }
    }

    /// A principal: profile, its `<handle>@web` emitter entity, and a personal context.
    struct Principal {
        profile: ProfileId,
        handle: String,
        emitter: EntityId,
        home: ContextId,
    }

    /// The handle is the FULL id: two UUIDv7s minted in one millisecond share leading bytes, so
    /// a truncated handle collides on `kb_profiles_handle_key` (the template's rule).
    async fn principal(pool: &PgPool) -> Principal {
        let id = Uuid::now_v7();
        let handle = format!("user-{id}");
        sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
            .bind(id)
            .bind(&handle)
            .execute(pool)
            .await
            .expect("seed profile");
        let emitter: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(id)
        .bind(format!("{handle}@web"))
        .fetch_one(pool)
        .await
        .expect("seed emitter entity");
        let home: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'home', 'Home') RETURNING id",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("seed personal context");
        Principal {
            profile: ProfileId::from(id),
            handle,
            emitter: EntityId::from(emitter),
            home: ContextId::from(home),
        }
    }

    /// An operator's sealed proof, minted through the real gate after `grant_governance`.
    async fn operator(pool: &PgPool) -> SystemAdmin {
        let op = principal(pool).await;
        test_support::grant_governance(pool, op.profile.uuid()).await;
        test_support::system_admin_proof_for(pool, op.profile.uuid()).await
    }

    /// A resource created through the REAL create path, homed in `owner`'s context.
    async fn resource_with_mode(
        pool: &PgPool,
        owner: &Principal,
        title: &str,
        mode: CreateMode,
    ) -> ResourceId {
        let origin = format!("test://{title}-{}", Uuid::now_v7());
        writes::create_resource_with_mode(
            pool,
            CreateParams {
                idempotency_key: None,
                title,
                origin_uri: &origin,
                body: "body under erasure",
                doc_type: "research",
                home: AnchorRef::context(owner.home),
                owner: owner.profile,
                originator: owner.profile,
                emitter: owner.emitter,
                properties: &[],
                chunks: None,
                sources: vec![],
            },
            EventContext::default(),
            mode,
        )
        .await
        .expect("create resource through the real path")
    }

    async fn resource(pool: &PgPool, owner: &Principal, title: &str) -> ResourceId {
        resource_with_mode(pool, owner, title, CreateMode::default()).await
    }

    /// A live blob committed through the REAL path (its bytes pre-registered in `store`), with a
    /// relation edge from it to each of `related`. Returns the blob and its pathname.
    async fn related_blob(
        pool: &PgPool,
        store: &InMemoryBlobStore,
        owner: &Principal,
        related: &[ResourceId],
    ) -> (BlobId, String) {
        let bytes = format!("blob bytes {}", Uuid::now_v7());
        let hash = format!("{:x}", sha2::Sha256::digest(bytes.as_bytes()));
        let pathname = blob_pathname(&hash);
        store.insert(pathname.clone());
        let blob = writes::commit_blob(
            pool,
            store,
            CommitBlobParams {
                id: BlobId::from(Uuid::now_v7()),
                home: AnchorRef::context(owner.home),
                owner: owner.profile,
                originator: None,
                content_hash: hash,
                content_type: "image/png".to_owned(),
                content_bytes: bytes.len() as i64,
                max_bytes: 10 * 1024 * 1024,
                allowlist: &["image/png".to_owned()][..],
                emitter: owner.emitter,
            },
        )
        .await
        .expect("the blob commits through the real path");
        for r in related {
            let mut conn = pool.acquire().await.expect("acquire");
            fire(
                &mut conn,
                SeedAction::RelationshipAssert {
                    src: AnchorRef::blob(blob),
                    tgt: AnchorRef::resource(*r),
                    kind: EdgeKind::Contains,
                    polarity: EdgePolarity::Forward,
                    label: Some("attached"),
                    weight: 1.0,
                    home: EdgeHome::Context(owner.home),
                    emitter: owner.emitter,
                },
            )
            .await
            .expect("the blob relation asserts through the real path");
        }
        (blob, pathname)
    }

    fn request(resource: ResourceId, blobs: &[BlobId]) -> ResourceErasureRequest<'_> {
        ResourceErasureRequest {
            resource,
            also_strike_blobs: blobs,
            surface: Surface::ApiHttp,
        }
    }

    async fn execute(
        pool: &PgPool,
        admin: &SystemAdmin,
        resource: ResourceId,
    ) -> ApiResult<ResourceErasureOutcome> {
        execute_resource_erasure(pool, None, admin, request(resource, &[])).await
    }

    async fn events_of(pool: &PgPool, kind: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
              WHERE t.name = $1",
        )
        .bind(kind)
        .fetch_one(pool)
        .await
        .expect("count events")
    }

    async fn erased_at(
        pool: &PgPool,
        resource: ResourceId,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1")
            .bind(resource.uuid())
            .fetch_one(pool)
            .await
            .expect("resource row")
    }

    fn completed(outcome: ResourceErasureOutcome) -> ResourceErasureCompletion {
        match outcome {
            ResourceErasureOutcome::Completed(c) => c,
            other => panic!("the act must complete, got {other:?}"),
        }
    }

    fn refused(outcome: ResourceErasureOutcome) -> ResourceErasureRefusal {
        match outcome {
            ResourceErasureOutcome::Refused(r) => r,
            other => panic!("the act must be refused, got {other:?}"),
        }
    }

    fn assert_no_raise_literal(text: &str) {
        for fragment in RAISE_FRAGMENTS {
            assert!(
                !text.contains(fragment),
                "a response carries raise text {fragment:?}: {text}"
            );
        }
    }

    /// ── WITNESS: the charter refusal ────────────────────────────────────────────────────────
    /// FAILS IF a charter resource is not refused-and-recorded, or its detail is not the fixed
    /// map-grain text.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_charter_resource_is_refused_recorded_with_the_fixed_detail(pool: PgPool) {
        bootseed::seed_system(&pool).await.expect("boot seed");
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let telos = {
            let mut conn = pool.acquire().await.expect("acquire");
            fire(
                &mut conn,
                SeedAction::CogmapGenesis {
                    name: "charter-map",
                    telos_title: "charter telos",
                    charter: &[],
                    cogmap_id: None,
                    telos_resource_id: None,
                    owner: owner.profile,
                    emitter: owner.emitter,
                },
            )
            .await
            .expect("cogmap genesis")
            .cogmap_genesis()
            .expect("genesis mints the telos")
            .1
        };

        let refusal = refused(execute(&pool, &op, telos).await.expect("the door answers"));
        assert_eq!(
            refusal.reason,
            ResourceErasureRefusalReason::CharterResource
        );
        assert_eq!(
            refusal.detail,
            Some(ResourceErasureRefusalDetail::MapGrainErasureTask)
        );

        let (reason, detail, correlation): (String, String, Uuid) = sqlx::query_as(
            "SELECT payload->>'reason', payload->>'detail', correlation_id FROM kb_events \
              WHERE id = $1",
        )
        .bind(refusal.event_id)
        .fetch_one(&pool)
        .await
        .expect("the refusal is recorded");
        assert_eq!(reason, "charter_resource");
        assert_eq!(detail, MAP_GRAIN_ERASURE_DETAIL);
        assert_eq!(correlation, refusal.request_reference);
        assert!(erased_at(&pool, telos).await.is_none());
        assert_eq!(events_of(&pool, "resource_erased").await, 0);
    }

    /// ── WITNESS 11: a repeat erasure ────────────────────────────────────────────────────────
    /// FAILS IF a second execute mints a second `resource_erased`, changes any projection row,
    /// or goes unrecorded.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_repeat_erasure_is_refused_already_erased_and_changes_nothing(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let r = resource(&pool, &owner, "repeat").await;
        completed(execute(&pool, &op, r).await.expect("the first act runs"));
        let before = temper_substrate::replay::dump_projections(&pool)
            .await
            .expect("dump");

        let refusal = refused(execute(&pool, &op, r).await.expect("the door answers"));
        assert_eq!(refusal.reason, ResourceErasureRefusalReason::AlreadyErased);
        assert_eq!(refusal.detail, None);

        let after = temper_substrate::replay::dump_projections(&pool)
            .await
            .expect("dump");
        assert_eq!(before, after, "the repeat changed the projection");
        let reason: String =
            sqlx::query_scalar("SELECT payload->>'reason' FROM kb_events WHERE id = $1")
                .bind(refusal.event_id)
                .fetch_one(&pool)
                .await
                .expect("the refusal is recorded");
        assert_eq!(reason, "already_erased");
        assert_eq!(
            events_of(&pool, "resource_erased").await,
            1,
            "exactly one resource_erased"
        );
    }

    /// ── WITNESS 11: the two former refusals complete ────────────────────────────────────────
    /// FAILS IF the service refuses a tombstone (made by the real soft delete) or an in-flight
    /// ingest (a segmented create not yet finalized).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_tombstone_and_an_in_flight_ingest_both_complete(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;

        let tombstone = resource(&pool, &owner, "tombstone").await;
        let mut tx = pool.begin().await.expect("begin");
        fire(
            &mut tx,
            SeedAction::ResourceDelete {
                resource: tombstone,
                emitter: owner.emitter,
            },
        )
        .await
        .expect("soft delete through the real path");
        tx.commit().await.expect("commit");
        let active: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
            .bind(tombstone.uuid())
            .fetch_one(&pool)
            .await
            .expect("row");
        assert!(!active, "the witness needs a real tombstone");

        let in_flight = resource_with_mode(
            &pool,
            &owner,
            "in-flight",
            CreateMode {
                defer: false,
                segmented: true,
            },
        )
        .await;
        let ingest: String =
            sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
                .bind(in_flight.uuid())
                .fetch_one(&pool)
                .await
                .expect("row");
        assert_eq!(
            ingest, "in_progress",
            "the witness needs an ingest in flight"
        );

        completed(
            execute(&pool, &op, tombstone)
                .await
                .expect("the tombstone erases"),
        );
        let flight = completed(
            execute(&pool, &op, in_flight)
                .await
                .expect("the ingest erases"),
        );
        assert!(erased_at(&pool, tombstone).await.is_some());
        assert!(erased_at(&pool, in_flight).await.is_some());
        assert!(
            flight
                .targets
                .iter()
                .any(|t| t.target == "kb_resources.ingest_state"),
            "the record names the ended ingest: {:?}",
            flight.targets
        );
        assert_eq!(events_of(&pool, "resource_erasure_refused").await, 0);
    }

    /// ── WITNESS: one reference per act ──────────────────────────────────────────────────────
    /// FAILS IF two acts share a request reference, or the returned reference is not the one
    /// the act's events are correlated by.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn two_acts_get_two_distinct_request_references(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let a = completed(
            execute(&pool, &op, resource(&pool, &owner, "first").await)
                .await
                .expect("first act"),
        );
        let b = completed(
            execute(&pool, &op, resource(&pool, &owner, "second").await)
                .await
                .expect("second act"),
        );
        assert_ne!(a.request_reference, b.request_reference);
        for c in [&a, &b] {
            let correlation: Uuid =
                sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = $1")
                    .bind(c.event_id)
                    .fetch_one(&pool)
                    .await
                    .expect("the completion event");
            assert_eq!(correlation, c.request_reference);
        }
    }

    /// ── WITNESS: no raise text reaches a caller ─────────────────────────────────────────────
    /// FAILS IF a refusal's rendered detail or an error string carries a raise literal. The
    /// scan reads the text a door renders (`detail.as_str()`, `ApiError`'s message), not a
    /// `Debug` of the outcome, whose variant names could never match. Not vacuous: the charter
    /// and already-erased refusals exist only because the SQL raised, and the two blob 400s are
    /// produced only by the classifier's arms over a raised act.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn no_response_or_error_contains_a_raise_literal(pool: PgPool) {
        bootseed::seed_system(&pool).await.expect("boot seed");
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let store = InMemoryBlobStore::default();
        let mut texts: Vec<String> = Vec::new();

        // Charter.
        let telos = {
            let mut conn = pool.acquire().await.expect("acquire");
            fire(
                &mut conn,
                SeedAction::CogmapGenesis {
                    name: "raise-map",
                    telos_title: "raise telos",
                    charter: &[],
                    cogmap_id: None,
                    telos_resource_id: None,
                    owner: owner.profile,
                    emitter: owner.emitter,
                },
            )
            .await
            .expect("cogmap genesis")
            .cogmap_genesis()
            .expect("telos")
            .1
        };
        let charter = refused(execute(&pool, &op, telos).await.expect("answers"));
        texts.extend(charter.detail.map(|d| d.as_str().to_string()));
        assert_eq!(
            texts.len(),
            1,
            "the charter refusal renders a detail to scan"
        );

        // Already erased, and a blob struck once through a completed act.
        let r1 = resource(&pool, &owner, "raise-r1").await;
        let r2 = resource(&pool, &owner, "raise-r2").await;
        let r3 = resource(&pool, &owner, "raise-r3").await;
        let (blob, _) = related_blob(&pool, &store, &owner, &[r1, r2]).await;
        completed(
            execute_resource_erasure(&pool, Some(&store), &op, request(r1, &[blob]))
                .await
                .expect("r1 erases, striking the blob"),
        );
        let repeat = refused(execute(&pool, &op, r1).await.expect("answers"));
        assert_eq!(repeat.reason, ResourceErasureRefusalReason::AlreadyErased);
        texts.extend(repeat.detail.map(|d| d.as_str().to_string()));
        assert_eq!(
            events_of(&pool, "resource_erasure_refused").await,
            2,
            "both raised"
        );

        // The blob again, through r2 (still related, already struck) and r3 (never related).
        let struck = execute_resource_erasure(&pool, Some(&store), &op, request(r2, &[blob]))
            .await
            .expect_err("an already-struck blob is refused");
        let unrelated = execute_resource_erasure(&pool, Some(&store), &op, request(r3, &[blob]))
            .await
            .expect_err("an unrelated blob is refused");
        for err in [&struck, &unrelated] {
            let ApiError::BadRequest(msg) = err else {
                panic!("a blob refusal is a 400, got {err:?}");
            };
            assert!(msg.contains(&blob.to_string()), "names the blob: {msg}");
            texts.push(err.to_string());
        }
        assert!(erased_at(&pool, r2).await.is_none() && erased_at(&pool, r3).await.is_none());

        // An unknown id, past the gate.
        let unknown = execute(&pool, &op, ResourceId::from(Uuid::now_v7()))
            .await
            .expect_err("an unknown id is not found");
        assert!(matches!(unknown, ApiError::NotFound(_)), "got {unknown:?}");
        texts.push(unknown.to_string());

        for text in &texts {
            assert_no_raise_literal(text);
        }
    }

    /// ── WITNESS: the classifier against the live SQL ────────────────────────────────────────
    /// FAILS IF a RAISE in the act or in `blob_delete` is reworded so the classifier no longer
    /// recognizes it (it would otherwise degrade to `Internal` silently).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_classifier_matches_the_live_sql_raises(pool: PgPool) {
        bootseed::seed_system(&pool).await.expect("boot seed");
        let owner = principal(&pool).await;
        let store = InMemoryBlobStore::default();
        let raw = |resource: Uuid, blobs: Vec<Uuid>| {
            let pool = pool.clone();
            let emitter = owner.emitter.uuid();
            let actor = owner.profile.uuid();
            async move {
                sqlx::query_scalar::<_, serde_json::Value>(
                    "SELECT resource_erasure_execute($1, $2, $3, $4, $5)",
                )
                .bind(resource)
                .bind(actor)
                .bind(emitter)
                .bind(Uuid::now_v7())
                .bind(blobs)
                .fetch_one(&pool)
                .await
            }
        };

        let telos = {
            let mut conn = pool.acquire().await.expect("acquire");
            fire(
                &mut conn,
                SeedAction::CogmapGenesis {
                    name: "pin-map",
                    telos_title: "pin telos",
                    charter: &[],
                    cogmap_id: None,
                    telos_resource_id: None,
                    owner: owner.profile,
                    emitter: owner.emitter,
                },
            )
            .await
            .expect("cogmap genesis")
            .cogmap_genesis()
            .expect("telos")
            .1
        };
        let err = raw(telos.uuid(), vec![]).await.expect_err("charter raises");
        assert_eq!(classify_act_error(&err), ActFailure::Charter);

        let r1 = resource(&pool, &owner, "pin-r1").await;
        let r2 = resource(&pool, &owner, "pin-r2").await;
        let r3 = resource(&pool, &owner, "pin-r3").await;
        let (blob, _) = related_blob(&pool, &store, &owner, &[r1, r2]).await;
        raw(r1.uuid(), vec![blob.uuid()]).await.expect("r1 erases");

        let err = raw(r1.uuid(), vec![]).await.expect_err("repeat raises");
        assert_eq!(classify_act_error(&err), ActFailure::AlreadyErased);

        let err = raw(r3.uuid(), vec![blob.uuid()])
            .await
            .expect_err("unrelated raises");
        assert_eq!(
            classify_act_error(&err),
            ActFailure::BlobNotInRemainder(Some(Uuid::from(blob)))
        );

        let err = raw(r2.uuid(), vec![blob.uuid()])
            .await
            .expect_err("struck raises");
        assert_eq!(
            classify_act_error(&err),
            ActFailure::BlobAlreadyStruck(Some(Uuid::from(blob)))
        );

        let err = raw(Uuid::now_v7(), vec![])
            .await
            .expect_err("unknown raises");
        assert_eq!(classify_act_error(&err), ActFailure::NotFound);
    }

    /// ── WITNESS: the husk short-circuit ─────────────────────────────────────────────────────
    /// FAILS IF the survey of an erased resource runs the plan (which still counts husk rows).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_survey_short_circuits_on_a_husk(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let r = resource(&pool, &owner, "husk").await;
        completed(execute(&pool, &op, r).await.expect("erases"));

        let survey = survey_resource_erasure(&pool, &op, r)
            .await
            .expect("answers");
        assert!(survey.already_erased);
        assert_eq!(survey.plan, None, "no plan on a husk");
        assert_eq!(
            survey.completion_fields,
            vec![],
            "an act under the ledger exception leaves nothing to complete"
        );
    }

    /// ── WITNESS 15 at the doors: the completion pass ───────────────────────────────────────
    /// FAILS IF a husk erased under cut 1 cannot be completed through the doors, or if the
    /// survey's `completion_fields` is not what the pass then rewrites, or if a completed husk is
    /// not refused, on the record, as `already_erased`. The cut-1 husk is made as the cut-1 act
    /// made one: the projection-side body under a record naming no `redacted_fields`.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_cut1_husk_surveys_its_completion_and_completes_once(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let r = resource(&pool, &owner, "cut-1 husk").await;
        sqlx::query(
            "SELECT _resource_erasure_apply_redaction($1, _event_append('resource_erased', $2, NULL, NULL, \
                 jsonb_build_object('subject_table', 'kb_resources', 'subject_id', $1, \
                                    'ledger_remainder', resource_erasure_survey_plan($1)->'redacted_fields')))",
        )
        .bind(r.uuid())
        .bind(owner.emitter.uuid())
        .execute(&pool)
        .await
        .expect("a cut-1 husk");

        let survey = survey_resource_erasure(&pool, &op, r)
            .await
            .expect("answers");
        assert!(survey.already_erased);
        assert_eq!(survey.plan, None);
        assert!(
            survey
                .completion_fields
                .iter()
                .any(|f| f.paths.iter().any(|p| p == "title")),
            "the survey names the title the ledger still carries: {:?}",
            survey.completion_fields
        );

        let completion = completed(execute(&pool, &op, r).await.expect("the pass runs"));
        assert_eq!(completion.redacted_fields, survey.completion_fields);
        assert!(completion.folded_edges.is_empty() && completion.targets.is_empty());
        assert_eq!(events_of(&pool, "resource_erased").await, 2);

        let after = survey_resource_erasure(&pool, &op, r)
            .await
            .expect("answers");
        assert_eq!(after.completion_fields, vec![], "nothing is left");
        let refusal = refused(execute(&pool, &op, r).await.expect("the door answers"));
        assert_eq!(refusal.reason, ResourceErasureRefusalReason::AlreadyErased);
        assert_eq!(events_of(&pool, "resource_erased").await, 2);
    }

    /// ── WITNESS: another principal's edge is named, with its author ─────────────────────────
    /// FAILS IF an edge (or an edge-owned property) another principal asserted goes unnamed, or
    /// the owner's own edge — asserted from a SECOND surface entity — is misnamed as another
    /// principal's (the profile-to-profile comparison).
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_survey_names_another_principals_edge_and_its_author(pool: PgPool) {
        let owner = principal(&pool).await;
        let other = principal(&pool).await;
        let op = operator(&pool).await;
        let r = resource(&pool, &owner, "authored").await;
        let s = resource(&pool, &other, "other-notes").await;
        let t = resource(&pool, &owner, "owner-notes").await;

        let foreign = writes::assert_relationship(
            &pool,
            AssertParams {
                src: s,
                tgt: r,
                kind: EdgeKind::LeadsTo,
                polarity: EdgePolarity::Forward,
                label: Some("another principal's words"),
                weight: 1.0,
                home: other.home,
                emitter: other.emitter,
            },
        )
        .await
        .expect("the other principal's edge");
        let foreign_prop = writes::assert_keyed_property_with(
            &pool,
            PropertyOwner::edge(foreign),
            "note",
            &serde_json::json!("their text"),
            1.0,
            other.emitter,
            EventContext::default(),
        )
        .await
        .expect("the other principal's edge property");

        // The owner's own edge, from the owner's `@cli` entity: a second surface, same principal.
        let owner_cli: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(owner.profile.uuid())
        .bind(format!("{}@cli", owner.handle))
        .fetch_one(&pool)
        .await
        .expect("owner cli entity");
        let own = writes::assert_relationship(
            &pool,
            AssertParams {
                src: r,
                tgt: t,
                kind: EdgeKind::LeadsTo,
                polarity: EdgePolarity::Forward,
                label: Some("the owner's words"),
                weight: 1.0,
                home: owner.home,
                emitter: EntityId::from(owner_cli),
            },
        )
        .await
        .expect("the owner's edge");

        let plan = survey_resource_erasure(&pool, &op, r)
            .await
            .expect("answers")
            .plan
            .expect("a live resource has a plan");
        assert!(plan.edges.contains(&foreign) && plan.edges.contains(&own));
        assert_eq!(
            plan.other_author_edges,
            vec![OtherAuthorEdge {
                edge_id: foreign,
                author: other.profile,
                folded: false,
            }],
            "only the other principal's edge is named"
        );
        assert_eq!(
            plan.other_author_edge_properties,
            vec![OtherAuthorEdgeProperty {
                property_id: foreign_prop,
                edge_id: foreign,
                author: other.profile,
                folded: false,
            }]
        );
    }

    /// ── WITNESS: a blob's co-link holder is named ───────────────────────────────────────────
    /// FAILS IF the survey does not name the other resource linking a related blob, or names
    /// the surveyed resource itself as a holder.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_survey_names_a_blobs_co_link_holder(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let store = InMemoryBlobStore::default();
        let r = resource(&pool, &owner, "co-link-r").await;
        let holder = resource(&pool, &owner, "co-link-holder").await;
        let (shared, _) = related_blob(&pool, &store, &owner, &[r, holder]).await;
        let (alone, _) = related_blob(&pool, &store, &owner, &[r]).await;

        let plan = survey_resource_erasure(&pool, &op, r)
            .await
            .expect("answers")
            .plan
            .expect("a plan");
        let mut links = plan.blob_co_links;
        links.sort_by_key(|l| l.blob_id.uuid());
        let mut expected = vec![
            BlobCoLinks {
                blob_id: shared,
                holders: vec![holder],
            },
            BlobCoLinks {
                blob_id: alone,
                holders: vec![],
            },
        ];
        expected.sort_by_key(|l| l.blob_id.uuid());
        assert_eq!(links, expected);
    }

    /// ── WITNESS: the post-commit byte release ───────────────────────────────────────────────
    /// FAILS IF a released strike's bytes survive the call (the release did not run after the
    /// commit), or if the fence, deriving the same delete from the `resource_erased` payload,
    /// fails it or leaves it outstanding — the provider delete of an absent object is a no-op.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_listed_blob_strike_releases_its_bytes_and_the_fence_has_nothing_left(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let store = InMemoryBlobStore::default();
        let r = resource(&pool, &owner, "strike").await;
        let (blob, pathname) = related_blob(&pool, &store, &owner, &[r]).await;
        assert!(
            store.contains(&pathname),
            "the witness needs the bytes present"
        );

        let completion = completed(
            execute_resource_erasure(&pool, Some(&store), &op, request(r, &[blob]))
                .await
                .expect("the act completes"),
        );
        assert_eq!(
            completion.blob_strikes,
            vec![BlobStrikeOutcome {
                blob_id: Uuid::from(blob),
                released: true,
            }]
        );
        assert!(
            !store.contains(&pathname),
            "the bytes are released after the commit, before any fence tick"
        );

        let summary = erasure_fence_service::drain(&pool, &store)
            .await
            .expect("the fence drains");
        assert_eq!(summary.seeded, 1, "the fence derives the same delete");
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.unparseable_verdicts, 0);
        let outstanding: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_erasure_blob_deletes \
              WHERE status IN ('pending', 'in_progress', 'waiting_for_retry', 'dead')",
        )
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(outstanding, 0, "nothing left to drain");
        let again = erasure_fence_service::drain(&pool, &store)
            .await
            .expect("drains");
        assert_eq!(again.claimed, 0);
    }

    /// ── WITNESS: the fence seeds from a `resource_erased` payload, unassisted ────────────────
    /// The act runs with NO store, so nothing releases the bytes after the commit: the only
    /// thing that can delete them is the fence deriving the delete from the `resource_erased`
    /// event's `targets`.
    /// FAILS IF the fence's seed scan drops `'resource_erased'` from its event-type `IN (...)`
    /// (nothing seeds, `seeded` is 0, the bytes stay), or if it seeds under any event other than
    /// the act's own, or if the drain does not delete the pathname.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn the_fence_seeds_a_resource_erased_strike_and_the_drain_deletes_the_bytes(
        pool: PgPool,
    ) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let store = InMemoryBlobStore::default();
        let r = resource(&pool, &owner, "fence-seed").await;
        let (blob, pathname) = related_blob(&pool, &store, &owner, &[r]).await;

        let completion = completed(
            execute_resource_erasure(&pool, None, &op, request(r, &[blob]))
                .await
                .expect("the act completes"),
        );
        assert!(
            store.contains(&pathname),
            "no store was passed, so nothing released the bytes after the commit"
        );

        let summary = erasure_fence_service::drain(&pool, &store)
            .await
            .expect("the fence drains");
        assert_eq!(summary.seeded, 1, "seeded from the resource_erased payload");
        assert_eq!(summary.unparseable_verdicts, 0);
        assert_eq!(summary.deleted, 1, "the drain struck the pathname");
        assert!(!store.contains(&pathname), "the bytes are gone");

        let seeded_under: Vec<Uuid> = sqlx::query_scalar(
            "SELECT erasure_event_id FROM kb_erasure_blob_deletes WHERE pathname = $1",
        )
        .bind(&pathname)
        .fetch_all(&pool)
        .await
        .expect("fence rows");
        assert_eq!(
            seeded_under,
            vec![completion.event_id],
            "one queue row, keyed by the resource_erased event"
        );
    }

    /// ── WITNESS: a remainder-only related blob never seeds ──────────────────────────────────
    /// Two related blobs; the operator lists neither. Both stay in `remainder` (D8), so the
    /// payload's `targets` carry no blob and the fence has nothing to derive.
    /// FAILS IF the act strikes an unlisted blob: the remainder would stop naming it, `targets`
    /// would carry its verdict, and the fence would seed and delete its bytes.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn an_unlisted_related_blob_stays_in_the_remainder_and_is_never_seeded(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let store = InMemoryBlobStore::default();
        let r = resource(&pool, &owner, "fence-remainder").await;
        let (first_blob, first) = related_blob(&pool, &store, &owner, &[r]).await;
        let (second_blob, second) = related_blob(&pool, &store, &owner, &[r]).await;

        let completion = completed(
            execute_resource_erasure(&pool, None, &op, request(r, &[]))
                .await
                .expect("the act completes"),
        );
        let mut named = related_blob_ids(&completion.remainder).expect("the plan's shape");
        named.sort();
        let mut both = vec![Uuid::from(first_blob), Uuid::from(second_blob)];
        both.sort();
        assert_eq!(
            named, both,
            "the remainder's kb_blobs entries are exactly the two related blobs"
        );
        assert!(
            completion.targets.iter().all(|t| t.target != "kb_blobs"),
            "an unlisted blob is never a target"
        );

        let summary = erasure_fence_service::drain(&pool, &store)
            .await
            .expect("the fence drains");
        assert_eq!(summary.seeded, 0, "nothing derives from the remainder");
        assert_eq!(summary.claimed, 0);
        assert_eq!(summary.deleted, 0);
        assert!(store.contains(&first) && store.contains(&second));
    }

    /// ── WITNESS: a committed act never answers as a failure ─────────────────────────────────
    /// The provider misbehaves AFTER the act commits: its delete errors, or it never returns.
    /// FAILS IF either turns the committed act into an error, or into a call that does not
    /// return (the hang arm runs under a 200ms release bound inside a 30s outer limit, so an
    /// unbounded release trips the outer bound): the answer must be `Completed`, carrying the
    /// minted reference and the event id of the `resource_erased` correlated by it, with the
    /// strike labelled from the act's own targets and the bytes left for the fence.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_failed_or_hung_post_commit_release_still_answers_completed(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        for mode in [BadDelete::Fails, BadDelete::Hangs] {
            let store = BadDeleteStore {
                inner: InMemoryBlobStore::default(),
                mode,
            };
            let r = resource(&pool, &owner, "bad-release").await;
            let (blob, pathname) = related_blob(&pool, &store.inner, &owner, &[r]).await;

            let outcome = tokio::time::timeout(
                Duration::from_secs(30),
                execute_with_release_timeout(
                    &pool,
                    Some(&store),
                    &op,
                    request(r, &[blob]),
                    Duration::from_millis(200),
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("{mode:?}: the door must answer, not wait on the provider"))
            .unwrap_or_else(|e| panic!("{mode:?}: a committed act is not an error, got {e:?}"));
            let completion = completed(outcome);

            let (kind, correlation): (String, Uuid) = sqlx::query_as(
                "SELECT t.name, e.correlation_id FROM kb_events e \
                   JOIN kb_event_types t ON t.id = e.event_type_id WHERE e.id = $1",
            )
            .bind(completion.event_id)
            .fetch_one(&pool)
            .await
            .expect("the completion's event");
            assert_eq!(kind, "resource_erased", "{mode:?}");
            assert_eq!(correlation, completion.request_reference, "{mode:?}");
            assert_eq!(
                completion.blob_strikes,
                vec![BlobStrikeOutcome {
                    blob_id: Uuid::from(blob),
                    released: true,
                }],
                "{mode:?}"
            );
            assert!(erased_at(&pool, r).await.is_some(), "{mode:?}");
            assert!(
                store.inner.contains(&pathname),
                "{mode:?}: the failed release left the bytes for the fence"
            );
        }
        assert_eq!(events_of(&pool, "resource_erasure_refused").await, 0);
    }

    /// ── WITNESS: a list naming one blob twice ───────────────────────────────────────────────
    /// FAILS IF an operator's duplicate reaches the act (the SQL would strike the blob, then
    /// raise on its second mention and answer the false "struck by an earlier act" 400), or if
    /// the 400 appends anything. A caller who is not a system admin never reaches this check:
    /// the door answers 404 first (`admin_resource_erasure_surface_test`). The last call — the
    /// same blob listed once — completes.
    #[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
    async fn a_blob_listed_twice_is_refused_before_the_act(pool: PgPool) {
        let owner = principal(&pool).await;
        let op = operator(&pool).await;
        let store = InMemoryBlobStore::default();
        let r = resource(&pool, &owner, "duplicate").await;
        let (blob, _) = related_blob(&pool, &store, &owner, &[r]).await;

        let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
            .fetch_one(&pool)
            .await
            .expect("count");
        let err = execute_resource_erasure(&pool, Some(&store), &op, request(r, &[blob, blob]))
            .await
            .expect_err("an operator's duplicate is refused");
        let ApiError::BadRequest(msg) = &err else {
            panic!("a duplicate is a 400, got {err:?}");
        };
        assert!(msg.contains("listed more than once"), "{msg}");
        assert!(msg.contains(&blob.to_string()), "{msg}");
        assert_no_raise_literal(msg);
        let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(
            events_before, events_after,
            "the operator's 400 appended nothing"
        );
        assert!(erased_at(&pool, r).await.is_none());

        completed(
            execute_resource_erasure(&pool, Some(&store), &op, request(r, &[blob]))
                .await
                .expect("listed once, the act completes"),
        );
    }
}
