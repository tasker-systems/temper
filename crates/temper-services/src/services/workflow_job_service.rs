//! Thin wrappers over the `kb_workflow_jobs` SQL primitives (goal 019f3220). `DbBackend` composes
//! these into the dispatch tick; tests exercise them directly. Auth is NOT here — these are queue
//! primitives; the dispatch command that composes them carries the auth gate.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::auditor::{AuditJobPayload, ClaimedAuditJob};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, CorrelationId, ProfileId};
use temper_core::types::workflow_job::{
    AnchorJobPayload, ClaimedAnchorJob, ClaimedEmbedJob, ClaimedJob, ClaimedSystemJob,
};

/// Enqueue a payload-less job for `(cogmap, persona, dispatch_type)`. Returns `Some(id)` when a new
/// row was created, `None` when one is already in-flight for the tuple (the single-flight dedup).
pub async fn enqueue(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    enqueue_with_payload(
        pool,
        cogmap_id,
        persona,
        dispatch_type,
        serde_json::Value::Object(serde_json::Map::new()),
    )
    .await
}

/// Enqueue a job for `(cogmap, persona, dispatch_type)` carrying `payload` into
/// `kb_workflow_jobs.payload`. Returns `Some(id)` when a new row was created, `None` when one is
/// already in-flight for the tuple.
///
/// **`None` is not "nothing to do" for a payload-carrying caller.** The single-flight index
/// (`migrations/20260705000001_workflow_jobs.sql:43-45`) is keyed on the tuple alone, so an
/// in-flight job with a *stale* payload swallows this one silently — which is exactly why the
/// auditor groups its whole finding list into ONE enqueue per cogmap instead of calling this once
/// per finding (spec §6.1). A caller that fans out over a payload must group first.
pub async fn enqueue_with_payload(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
    payload: serde_json::Value,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue($1, $2, $3, $4) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
        payload,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable jobs, leasing each for `lease_seconds`, stamping each claimed row
/// with the `correlation` of the dispatch tick that claimed it (`None` → NULL; the tick sent no
/// `x-steward-correlation-id`). The stamp is what `invocation_open` later inherits, so a session's
/// invocation joins back to its tick without the agent threading anything.
///
/// `principal` does two things, exactly as it does in [`claim_audit`]: it is the claimant stamped
/// into `claimed_by_profile_id`, AND it scopes the claimable set to
/// `steward_candidate_cogmaps(principal)`. Passing this tick's own `self.profile_id` cannot narrow
/// what the tick just enqueued — spec §3.3 is why: the sweep that enqueues already runs as the same
/// principal against `steward_authorable_cogmaps`, which is candidates ∩ authorable, strictly
/// narrower than the candidate set the claim filters through. Before this parameter existed the
/// steward's claim passed no principal at all, so `claimed_by_profile_id` defaulted NULL on every
/// steward job — the asymmetry [`claim_audit`]'s own doc comment names, and which `20260724000130`
/// recorded as fixed for the auditor but not for the steward.
pub async fn claim(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
    correlation: Option<CorrelationId>,
    principal: ProfileId,
) -> ApiResult<Vec<ClaimedJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", cogmap_id AS "cogmap_id!: Uuid", attempts AS "attempts!: i32"
          FROM workflow_job_claim($1, $2, $3, $4, $5, $6)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
        correlation.map(|c| c.uuid()),
        *principal,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedJob {
            id: r.id,
            cogmap_id: r.cogmap_id,
            attempts: r.attempts,
        })
        .collect())
}

/// Claim up to `limit` claimable jobs and read each one's payload back as an [`AuditJobPayload`] —
/// the auditor twin of [`claim`], the way [`claim_resource`] is the resource twin.
///
/// It is a separate function rather than a `payload` field on [`ClaimedJob`] because only the
/// auditor's jobs carry one: the steward's payload is always `{}`, and widening its wire type would
/// restale `openapi.json` and both SDKs to add a field that is permanently empty there.
///
/// A payload that does not deserialize is an [`ApiError::Internal`], not an empty finding list. The
/// only writer of these rows is [`enqueue_with_payload`] from this same crate, so a malformed one
/// means the shape drifted — and a claim that quietly reported "0 findings" would burn the job's
/// single-flight slot and its attempt on nothing, which is precisely the silent-drop failure this
/// whole grouping design exists to avoid.
///
/// **`principal` is REQUIRED here and optional on [`claim`], and that asymmetry is the fix, not an
/// oversight.** `workflow_job_claim` filtered on `(persona, dispatch_type, status)` alone, so a
/// caller claimed whatever was queued — and unlike the steward's payload-less `ClaimedJob`, an
/// auditor job's payload is the cogmap's **full finding-id list**. An unscoped claim therefore
/// disclosed ids from cogmaps the caller cannot read AND took those jobs `in_progress` away from the
/// principal that enqueued them. Passing the principal constrains the claimable set to
/// `steward_candidate_cogmaps(principal)` — the same predicate `audit_drift_sweep` enqueues through,
/// so a tick can only ever claim work its own sweep could have produced.
pub async fn claim_audit(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
    correlation: Option<CorrelationId>,
    principal: ProfileId,
) -> ApiResult<Vec<ClaimedAuditJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", cogmap_id AS "cogmap_id!: Uuid", attempts AS "attempts!: i32",
               payload AS "payload!: serde_json::Value"
          FROM workflow_job_claim($1, $2, $3, $4, $5, $6)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
        correlation.map(|c| c.uuid()),
        *principal,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let payload: AuditJobPayload = serde_json::from_value(r.payload).map_err(|e| {
                ApiError::Internal(format!(
                    "workflow job {} has an unreadable citation-audit payload: {e}",
                    r.id
                ))
            })?;
            Ok(ClaimedAuditJob {
                id: r.id,
                cogmap_id: r.cogmap_id,
                attempts: r.attempts,
                citations: payload.citations,
            })
        })
        .collect()
}

/// Transition the one **in-flight** job for the tuple → done, and only if `principal` is the one
/// that claimed it. Returns the job id if such a job existed, `None` otherwise.
///
/// The auditor's completion primitive, deliberately separate from [`complete`] rather than a
/// widening of it: [`complete`] is the steward's, it rides `steward_advance_watermark`, and its
/// broader `status IN ('pending','in_progress','waiting_for_retry')` transition is load-bearing
/// there. Two narrowings here, each named by what it prevents (the full argument lives beside the
/// SQL, `migrations/20260724000130_audit_drift_sweep.sql`):
///
/// * `status = 'in_progress'` — [`complete`] also completes a **pending** job, so a caller could
///   terminate one that had never been dispatched and suppress a cogmap's auditing indefinitely.
/// * `claimed_by_profile_id = principal` — completing someone else's in-flight job frees the
///   single-flight slot mid-session, and the next tick enqueues and claims a **second concurrent**
///   audit session over the same finding list, both appending to a trail that cannot retract.
///
/// `None` is not an error: an already-completed job, a reaped lease, or a manual audit outside the
/// dispatch loop all land there, and the session's written verdicts stand either way.
pub async fn complete_claimed(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
    principal: ProfileId,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete_claimed($1, $2, $3, $4) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
        *principal,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Transition the one active job for the tuple → done. Returns the job id if one was active.
pub async fn complete(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete($1, $2, $3) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Enqueue a resource-keyed job for `(resource, persona, dispatch_type)` — the resource twin of
/// [`enqueue`]. Returns `Some(id)` when a new row was created, `None` when one is already in-flight
/// for the tuple (the single-flight dedup, which also gives supersede-on-update for embed jobs).
pub async fn enqueue_resource(
    pool: &PgPool,
    resource_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue_resource($1, $2, $3, '{}'::jsonb) AS "id: Uuid""#,
        resource_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable resource-keyed jobs, leasing each for `lease_seconds` — the
/// resource twin of [`claim`], returning each job's `resource_id` (the embed scope).
pub async fn claim_resource(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
) -> ApiResult<Vec<ClaimedEmbedJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", resource_id AS "resource_id!: Uuid", attempts AS "attempts!: i32"
          FROM workflow_job_claim_resource($1, $2, $3, $4)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedEmbedJob {
            id: r.id,
            resource_id: r.resource_id,
            attempts: r.attempts,
        })
        .collect())
}

/// Transition the one active resource-keyed job for the tuple → done — the resource twin of
/// [`complete`]. Returns the job id if one was active.
pub async fn complete_resource(
    pool: &PgPool,
    resource_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete_resource($1, $2, $3) AS "id: Uuid""#,
        resource_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Enqueue a payload-less region-clock job for `(anchor, persona, dispatch_type)` — the anchor twin
/// of [`enqueue`] and [`enqueue_resource`]. Returns `Some(id)` when a new row was created, `None`
/// when one is already in-flight for the tuple.
///
/// **`None` is the point here, not a degraded outcome.** Every settling-worthy write on an anchor
/// enqueues; the single-flight index collapses them to one job, so N arrivals within a settling
/// window cost one materialization rather than N.
pub async fn enqueue_anchor<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    anchor: HomeAnchor,
    persona: &str,
    dispatch_type: &str,
    payload: AnchorJobPayload,
) -> ApiResult<Option<Uuid>> {
    // Exactly one of the pair is non-null — `ck_workflow_jobs_one_scope` enforces it, and
    // `HomeAnchor` being a closed two-variant enum is what makes this total rather than defaulted.
    let (cogmap, context) = match anchor {
        HomeAnchor::Cogmap(m) => (Some(m.uuid()), None),
        HomeAnchor::Context(c) => (None, Some(c.uuid())),
    };
    let payload = serde_json::to_value(payload).map_err(|e| ApiError::Internal(e.to_string()))?;
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue_anchor($1, $2, $3, $4, $5) AS "id: Uuid""#,
        cogmap,
        context,
        persona,
        dispatch_type,
        payload,
    )
    .fetch_one(executor)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable anchor-keyed jobs, leasing each for `lease_seconds` — the anchor
/// twin of [`claim`] and [`claim_resource`].
///
/// The `(cogmap_id, context_id)` pair the SQL returns is parsed into a [`HomeAnchor`] here, at the
/// boundary, so no caller downstream can hold an ambiguous scope. A row satisfying neither arm is a
/// genuine integrity failure — `ck_workflow_jobs_one_scope` should have made it unreachable — so it
/// escalates rather than being skipped: a silently-dropped job is a write that never settles and
/// leaves no trace, which is precisely what `projection-failure-is-never-silent` forbids.
pub async fn claim_anchor(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
) -> ApiResult<Vec<ClaimedAnchorJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", cogmap_id AS "cogmap_id?: Uuid",
               context_id AS "context_id?: Uuid", attempts AS "attempts!: i32",
               payload AS "payload!: serde_json::Value"
          FROM workflow_job_claim_anchor($1, $2, $3, $4)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let anchor = match (r.cogmap_id, r.context_id) {
                (Some(m), None) => HomeAnchor::Cogmap(CogmapId::from(m)),
                (None, Some(c)) => HomeAnchor::Context(ContextId::from(c)),
                _ => {
                    return Err(ApiError::Internal(format!(
                        "workflow job {} has an ambiguous anchor scope (cogmap={:?}, context={:?}) \
                         — ck_workflow_jobs_one_scope should make this unreachable",
                        r.id, r.cogmap_id, r.context_id
                    )))
                }
            };
            // A payload that will not deserialize escalates rather than defaulting to a system
            // emitter: silently re-attributing a settling to nobody is worse than not running it,
            // and the reaper will re-drive the job once the lease expires.
            let payload: AnchorJobPayload = serde_json::from_value(r.payload).map_err(|e| {
                ApiError::Internal(format!(
                    "workflow job {} carries an unreadable anchor payload: {e}",
                    r.id
                ))
            })?;
            Ok(ClaimedAnchorJob {
                id: r.id,
                anchor,
                emitter: payload.emitter,
                attempts: r.attempts,
            })
        })
        .collect()
}

/// Transition the one active anchor-keyed job for the tuple → done — the anchor twin of
/// [`complete`]. Returns the job id if one was active.
pub async fn complete_anchor<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    anchor: HomeAnchor,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let (cogmap, context) = match anchor {
        HomeAnchor::Cogmap(m) => (Some(m.uuid()), None),
        HomeAnchor::Context(c) => (None, Some(c.uuid())),
    };
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete_anchor($1, $2, $3, $4) AS "id: Uuid""#,
        cogmap,
        context,
        persona,
        dispatch_type,
    )
    .fetch_one(executor)
    .await?;
    Ok(id)
}

/// Enqueue a system-scoped job for `(persona, dispatch_type)`, carrying `payload`. This is the
/// anchorless twin of [`enqueue_anchor`] and [`enqueue_resource`]. Returns `Some(id)` when a new
/// row was created, and `None` when one is already in flight for the tuple
/// (`uq_workflow_jobs_in_flight_system`).
///
/// Only the system personas named in `ck_workflow_jobs_one_scope` may write an anchorless row.
/// Any other persona is refused by the table, by design: for the anchored families an anchorless
/// row means a NULL was passed by mistake.
pub async fn enqueue_system<P: serde::Serialize>(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    payload: &P,
) -> ApiResult<Option<Uuid>> {
    let payload = serde_json::to_value(payload).map_err(|e| ApiError::Internal(e.to_string()))?;
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue_system($1, $2, $3) AS "id: Uuid""#,
        persona,
        dispatch_type,
        payload,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable system-scoped jobs, leasing each for `lease_seconds`. This is the
/// anchorless twin of [`claim_anchor`]. The payload is parsed into `P` here, at the boundary.
///
/// A payload that will not deserialize escalates rather than being skipped, for the reason
/// [`claim_anchor`] gives: a silently dropped job leaves no trace, and the reaper re-drives it once
/// the lease expires.
pub async fn claim_system<P: serde::de::DeserializeOwned>(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
) -> ApiResult<Vec<ClaimedSystemJob<P>>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", attempts AS "attempts!: i32",
               payload AS "payload!: serde_json::Value"
          FROM workflow_job_claim_system($1, $2, $3, $4)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            // serde's message quotes the offending value, and a system job is one whose payload
            // must never carry content into a log. So the category goes in the error and the
            // value never does.
            let payload = serde_json::from_value(r.payload).map_err(|e| {
                ApiError::Internal(format!(
                    "workflow job {} carries an unreadable system payload ({:?} error)",
                    r.id,
                    e.classify()
                ))
            })?;
            Ok(ClaimedSystemJob {
                id: r.id,
                payload,
                attempts: r.attempts,
            })
        })
        .collect()
}

/// Transition one in-progress system-scoped job to done, **by job id**. Returns the id if the job
/// was in progress under this tuple. A pending job is not completed: that would cancel work never
/// dispatched, the narrowing `workflow_job_complete_claimed` made for the same reason.
///
/// The id is the only handle an anchorless job has. Every incumbent completer matches on its
/// anchor, and `NULL = NULL` is never true, so none of them can complete this job.
pub async fn complete_system(
    pool: &PgPool,
    job: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete_system($1, $2, $3) AS "id: Uuid""#,
        job,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Re-drive `dead` resource-keyed jobs — re-enqueue a fresh pending job per resource that has a dead
/// job, up to `limit` resources. Returns the re-enqueued resource ids. Skips any resource that already
/// has a live job (the underlying `ON CONFLICT DO NOTHING` against the resource in-flight index), so a
/// re-drive never creates a duplicate active job. The dead rows are left as an accountability trail.
pub async fn redrive_resource(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
) -> ApiResult<Vec<Uuid>> {
    let rows = sqlx::query!(
        r#"
        SELECT resource_id AS "resource_id!: Uuid"
          FROM workflow_job_redrive_resource($1, $2, $3)
        "#,
        persona,
        dispatch_type,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.resource_id).collect())
}

/// Reap expired-lease jobs → retry (or dead at max attempts). Returns the count reaped.
pub async fn reap(pool: &PgPool, error: &str) -> ApiResult<i32> {
    let n = sqlx::query_scalar!(r#"SELECT workflow_job_reap($1) AS "n!: i32""#, error)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
