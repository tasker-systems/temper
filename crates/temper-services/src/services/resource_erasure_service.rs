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
    ErasureAct, ErasureTargetOutcome, RecordedRefusalReason, RedactedEventFields,
    ResourceErasureRefusalReason,
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
    operator_gate(pool, operator).await?;
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
    queue_region_settling(pool, attempt.resource).await;

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
/// The settling runs as the **system actor**, never the operator. It is system-computed work, and
/// the events it writes (`region_materialized`, `salience_refreshed`) are domain events anchored on
/// the context, which the change feed serves with their emitter as actor. Run under the operator,
/// they would name who erased to every reader of the context, which the owner ruled an admin act
/// must never do (`_event_is_admin_act`, migration 20261116000010).
///
/// **Never fails the act**, as `DbBackend::queue_region_clocks` never fails a write: the act has
/// committed, and a failed enqueue leaves the regions to the next write that reaches the anchor.
pub(super) async fn queue_region_settling(pool: &PgPool, resource: ResourceId) {
    let emitter = match temper_substrate::readback::system_actor(pool).await {
        Ok((_, emitter)) => emitter,
        Err(e) => {
            tracing::warn!(
                resource = %resource.uuid(),
                error = %e,
                "failed to resolve the system actor for an erased resource's region settling; regions are stale until the next write re-drives them"
            );
            return;
        }
    };
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

/// What a demoted operator is told by every erasure door: the sentence the HTTP gate
/// (`require_erasure_operator`) answers a caller who is not a system admin with, so the doors'
/// refusal to a demoted admin is the one a non-admin gets.
pub(crate) const OPERATOR_REFUSAL: &str = "not found";

/// The erasure doors' admin re-check on an act's transaction, before its first write. The door's
/// `&SystemAdmin` proof was minted on the pool; an erasure is irreversible, so the operator's
/// governance row is read again `FOR KEY SHARE` (`auth::is_system_admin_locked`), held to the act's
/// commit: a demotion committed since refuses the act, one in flight makes it wait, then refuses.
///
/// Lock order: governance first, then the act's own locks (its resource act-queue advisory locks,
/// the resource and edge rows, the home context; the person act also writes its subject's
/// standing). A demotion locks the demoted admin's standing row and the governance pair, and no
/// erasure act takes a governance row, so the two wait in a cycle only if the person act's
/// subject were its own operator, whose demotion it would then outlive or refuse: one order
/// either way.
///
/// The wait is unbounded, as the acts' own are (they pin `lock_timeout = 0`, migration
/// `20261015100000`): a lock-wait bound set later on the pool or role must not turn a demotion in
/// flight into a busy refusal instead of this door's answer. `SET LOCAL`, so it ends with the
/// transaction.
///
/// The database error of the read itself (a deadlock, say) is returned as is, so an act's retry
/// loop classifies it as it classifies the act's own.
pub(crate) async fn operator_still_holds(
    tx: &mut sqlx::PgConnection,
    operator: ProfileId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!("SELECT set_config('lock_timeout', '0', true)")
        .fetch_one(&mut *tx)
        .await?;
    crate::auth::is_system_admin_locked_raw(tx, operator).await
}

/// [`operator_still_holds`] for a door with no retry loop: the refusal on `false`.
pub(crate) async fn operator_holds(
    tx: &mut sqlx::PgConnection,
    operator: ProfileId,
) -> ApiResult<()> {
    if operator_still_holds(tx, operator).await? {
        Ok(())
    } else {
        Err(operator_refusal(operator))
    }
}

/// The refusal of a demoted operator, with the `warn` line the HTTP gate writes for a non-admin,
/// so a stale proof's refusal leaves the same trace at the default filter.
pub(crate) fn operator_refusal(operator: ProfileId) -> ApiError {
    tracing::warn!(
        profile_id = %operator,
        "erasure door refused an operator who is no longer a system admin; answered 404, nothing \
         recorded on the ledger"
    );
    ApiError::NotFound(OPERATOR_REFUSAL.to_string())
}

/// The operator's standing at a door's entry, before it validates anything or records a refusal:
/// a demoted operator holding a stale proof is told what a non-admin is told, never a `400` or
/// `404` about the request, and no refusal is recorded in their name (a demotion that commits
/// after the act's re-check, while a refused act's refusal is being recorded, can still leave
/// that one record, a ledger row and nothing more). A plain read: it orders the refusals and holds
/// nothing. The act's own transaction asks again under the lock
/// ([`operator_holds`]), which is the check that holds to the commit.
pub(crate) async fn operator_gate(pool: &PgPool, operator: ProfileId) -> ApiResult<()> {
    if crate::services::access_service::is_system_admin(pool, operator).await? {
        Ok(())
    } else {
        Err(operator_refusal(operator))
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
        // Each attempt is its own transaction, the operator re-checked first under its lock.
        let mut tx = pool.begin().await?;
        let result = match operator_still_holds(&mut tx, attempt.operator).await {
            Ok(false) => return Err(operator_refusal(attempt.operator)),
            Ok(true) => {
                sqlx::query_scalar!(
                    r#"SELECT resource_erasure_execute($1, $2, $3, $4, $5)
                           AS "outcome: serde_json::Value""#,
                    attempt.resource.uuid(),
                    attempt.operator.uuid(),
                    attempt.emitter.uuid(),
                    attempt.request_reference,
                    &blobs[..],
                )
                .fetch_one(&mut *tx)
                .await
            }
            Err(err) => Err(err),
        };
        let result = match result {
            Ok(raw) => tx.commit().await.map(|()| raw),
            Err(err) => Err(err),
        };
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

/// Whether a failed call into `resource_erasure_execute` is one a fresh attempt can succeed at: a
/// deadlock, a raced edge fold, a remote source that gained or lost a citer, or a collision on the
/// redaction rows. The person act runs this act once per estate resource and retries on the same
/// classification (person-erasure design D8).
pub(super) fn is_retryable_act_error(err: &sqlx::Error) -> bool {
    matches!(classify_act_error(err), ActFailure::Retryable)
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
/// reference. The erasure and the block history scrub record their refusals here; the field
/// scrub, whose doors answer in their own vocabulary, records through [`record_refusal`].
///
/// THE `act` RULE: the erasure records `p_act = NULL` and NULL blocks, so its refusal payloads
/// carry neither key and stay the shape every earlier erasure refusal has (an absent `act` reads
/// as [`ErasureAct::Erasure`]). The block history scrub records `p_act = 'block_history_scrub'`
/// and the blocks the operator named, each a block of the resource: the scrub's doors check
/// membership before they record a refusal. The field scrub records `p_act = 'field_scrub'` and no
/// blocks. An empty `blocks` is passed as NULL; the SQL (`resource_erasure_refuse`, migrations
/// 20261003000210 and 20261018100020) accepts blocks only beside the block scrub's act.
pub(super) async fn refuse(
    pool: &PgPool,
    attempt: &Attempt,
    act: ErasureAct,
    blocks: &[Uuid],
    reason: ResourceErasureRefusalReason,
    detail: Option<ResourceErasureRefusalDetail>,
) -> ApiResult<ResourceErasureRefusal> {
    let event_id = record_refusal(pool, attempt, act, blocks, reason.into(), detail).await?;
    Ok(ResourceErasureRefusal {
        request_reference: attempt.request_reference,
        event_id,
        reason,
        detail,
    })
}

/// Append the refusal [`refuse`] describes, in the ledger's own vocabulary
/// ([`RecordedRefusalReason`], which serializes every value both door vocabularies share with the
/// same spelling), and return its event id.
pub(super) async fn record_refusal(
    pool: &PgPool,
    attempt: &Attempt,
    act: ErasureAct,
    blocks: &[Uuid],
    reason: RecordedRefusalReason,
    detail: Option<ResourceErasureRefusalDetail>,
) -> ApiResult<Uuid> {
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
    Ok(event_id)
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
mod tests;
