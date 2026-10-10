//! The field scrub's service layer — execute, survey and the family listing (field-grain scrub
//! spec 2026-10-09, S1, S2, S4, S5). The block history scrub's service
//! (`block_history_scrub_service`) is the template and its posture holds here unchanged: the gate
//! is the sealed [`SystemAdmin`] proof and nothing more, the request reference is minted here (one
//! `Uuid::now_v7()` per act), the act's raises are mapped through a closed classifier
//! (`classify_field_scrub_failure`), and nothing the SQL says reaches a response. The helpers the
//! acts share (the `Attempt`, the retry bound, the SQLSTATEs, the refusal recorder, the listed-id
//! refusal) live in `resource_erasure_service` and are shared by visibility, never copied.
//!
//! SQL commits, it does not decide legality: the listing, the plan, the refusals and the rewrite
//! live in `resource_field_scrub_families` / `resource_field_scrub_plan` /
//! `resource_field_scrub_survey` / `resource_field_scrub_execute` (migration 20261018100020). A
//! malformed request (a handle with a field that takes none, `property` without one, `properties`
//! with `clear`) and a handle that is not one of the resource's property families are 400s that
//! record nothing, whatever state the resource is in: both are checked before any refusal is
//! recorded, the handle through the act's own predicate (`_resource_field_scrub_request_fault`),
//! so a recorded refusal only ever names a real family of the resource (S4). A keep-mode request
//! with nothing prior is a 400 the act raises. A refusal the act raises (a charter, an erased
//! resource, a sentinel collision, a projection that disagrees) is recorded through
//! `resource_erasure_refuse` with the field scrub's act and no blocks.
//!
//! NO TEXT CROSSES (S1): a request names a field by kind and a family by an event id; every 400
//! here names only those; the listing, the plan and the record carry ids, counts, dates, profile
//! ids, JSON type names and paths. This module logs nothing of its own; an unexpected failure is
//! logged by [`ApiError::internal_scrubbed`] with the database's message, which the act's raises
//! keep to ids and kind names.
//!
//! The family flags are read after the listing, through `resource_field_scrub_family_flags` (the
//! sensitivity sweep's door, migration 20261018100030); the act never reads them. The HTTP doors
//! call straight into [`execute_field_scrub`] / [`survey_field_scrub`] /
//! [`list_field_scrub_families`]; this module carries no HTTP types.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::{
    FieldScrubClearingEvent, FieldScrubFamilies, FieldScrubFamily, FieldScrubHeldPaths,
    FieldScrubPlan, FieldScrubRefusalReason, FieldScrubSurvey,
};
use temper_core::types::ids::{EventId, PropertyId, ResourceId};
use temper_substrate::payloads::{ErasureAct, RedactedEventFields, ScrubFieldKind, ScrubbedField};
use temper_substrate::writes::resolve_emitter;
use temper_workflow::operations::Surface;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};
use crate::services::resource_erasure_service::{
    between, erased_state, listed_refusal, record_refusal, Attempt, ResourceErasureRefusalDetail,
    DEADLOCK_DETECTED, MAX_ACT_RETRIES, RAISE_EXCEPTION, RESOURCE_NOT_FOUND,
};

/// The prefix of every raise in `resource_field_scrub_execute` (migration 20261018100020).
const EXECUTE_RAISE_PREFIX: &str = "resource_field_scrub_execute: ";

/// The prefix of every raise in `resource_field_scrub_plan`, which the survey calls.
const PLAN_RAISE_PREFIX: &str = "resource_field_scrub_plan: ";

/// One execute or survey request. The operator is not here (the [`SystemAdmin`] proof names it),
/// and neither is the request reference: the service mints it.
#[derive(Debug, Clone, Copy)]
pub struct FieldScrubRequest {
    pub resource: ResourceId,
    pub field: ScrubFieldKind,
    /// The family's handle; required for [`ScrubFieldKind::Property`] and refused otherwise.
    pub family: Option<Uuid>,
    /// Clear today's value first (S2). Refused for [`ScrubFieldKind::Properties`].
    pub clear: bool,
    /// Where the request came from; the act and any refusal are attributed through it.
    pub surface: Surface,
}

impl FieldScrubRequest {
    /// The field as the record names it.
    fn scrubbed_field(&self) -> ScrubbedField {
        ScrubbedField {
            kind: self.field,
            family: self.family.map(EventId::from),
        }
    }
}

/// A completed scrub: ONE `resource_scrubbed` event stands behind these values.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldScrubCompletion {
    /// The server-minted reference the act is correlated by — the operator cites it.
    pub request_reference: Uuid,
    pub event_id: Uuid,
    pub field: ScrubbedField,
    /// True when the act cleared today's value first.
    pub cleared: bool,
    /// The ledger paths the act rewrote to their sentinels.
    pub redacted_fields: Vec<RedactedEventFields>,
}

/// A recorded refusal: one `resource_erasure_refused` event naming the field scrub, nothing else
/// mutated.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldScrubRefusal {
    /// The server-minted reference the refusal event is correlated by.
    pub request_reference: Uuid,
    pub event_id: Uuid,
    pub reason: FieldScrubRefusalReason,
    pub detail: Option<ResourceErasureRefusalDetail>,
    pub field: ScrubbedField,
}

/// What an execute call did.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldScrubOutcome {
    Completed(FieldScrubCompletion),
    Refused(FieldScrubRefusal),
}

/// The jsonb `resource_field_scrub_execute` returns.
#[derive(Debug, serde::Deserialize)]
struct ExecuteOutcomeWire {
    event_id: Uuid,
    redacted_fields: Vec<RedactedEventFields>,
    cleared: bool,
}

/// The jsonb `resource_field_scrub_survey` returns: the listing (null on a charter or an erased
/// resource) and the plan. Each family's flags are read afterwards.
#[derive(Debug, serde::Deserialize)]
struct SurveyWire {
    families: Option<Vec<FieldScrubFamily>>,
    plan: PlanWire,
}

/// The jsonb `resource_field_scrub_plan` computes.
#[derive(Debug, serde::Deserialize)]
struct PlanWire {
    redacted_fields: Vec<RedactedEventFields>,
    kept: Vec<FieldScrubHeldPaths>,
    unreachable: Vec<FieldScrubHeldPaths>,
    folded_rows: Vec<PropertyId>,
    clears: Vec<FieldScrubClearingEvent>,
    refusal: Option<String>,
}

/// How one run of the act ended, once the classifier has read any failure.
#[derive(Debug)]
enum FieldScrubVerdict {
    Completed(ExecuteOutcomeWire),
    Refused(
        FieldScrubRefusalReason,
        Option<ResourceErasureRefusalDetail>,
    ),
}

/// A request the act refuses as malformed or foreign: a 400 that records nothing (S4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestFault {
    UnknownFieldKind,
    NeedsFamily,
    /// A handle with a field that takes none.
    TakesNoFamily,
    PropertiesCannotBeCleared,
    /// A handle that is not a property family handle of the resource; the id when the message
    /// carried a parseable one.
    ForeignFamily(Option<Uuid>),
}

/// The closed classification of a failed `resource_field_scrub_execute` (or `_plan`) call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldScrubFailure {
    Charter,
    AlreadyErased,
    SentinelCollision,
    ProjectionDisagrees,
    NotFound,
    Fault(RequestFault),
    NothingPrior,
    /// A deadlock.
    Retryable,
    Other,
}

/// The wire spelling of a field kind, as the act and the listing take it.
fn kind_name(kind: ScrubFieldKind) -> &'static str {
    match kind {
        ScrubFieldKind::Title => "title",
        ScrubFieldKind::OriginUri => "origin_uri",
        ScrubFieldKind::Property => "property",
        ScrubFieldKind::Properties => "properties",
    }
}

/// Execute the field scrub, as the operator `admin` names.
///
/// The [`SystemAdmin`] proof is the gate, and it ran where the proof was minted. The operator's
/// emitter resolves first. A malformed request is [`ApiError::BadRequest`] before the act runs; an
/// unknown resource is [`ApiError::NotFound`]; a handle that is not one of the resource's property
/// families is [`ApiError::BadRequest`], for every state of the resource, checked before the act
/// runs and before any refusal is recorded. A charter, an erased resource, a sentinel collision or
/// a projection that disagrees is then a recorded refusal naming the field scrub. A keep-mode
/// request with nothing prior is [`ApiError::BadRequest`]: nothing was scrubbed or recorded.
pub async fn execute_field_scrub(
    pool: &PgPool,
    admin: &SystemAdmin,
    request: FieldScrubRequest,
) -> ApiResult<FieldScrubOutcome> {
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

    check_request(pool, &request).await?;

    let wire = match run_act(pool, &attempt, &request).await? {
        FieldScrubVerdict::Completed(wire) => wire,
        FieldScrubVerdict::Refused(reason, detail) => {
            let event_id = record_refusal(
                pool,
                &attempt,
                ErasureAct::FieldScrub,
                &[],
                reason.into(),
                detail,
            )
            .await?;
            return Ok(FieldScrubOutcome::Refused(FieldScrubRefusal {
                request_reference: attempt.request_reference,
                event_id,
                reason,
                detail,
                field: request.scrubbed_field(),
            }));
        }
    };

    Ok(FieldScrubOutcome::Completed(FieldScrubCompletion {
        request_reference: attempt.request_reference,
        event_id: wire.event_id,
        field: request.scrubbed_field(),
        cleared: wire.cleared,
        redacted_fields: wire.redacted_fields,
    }))
}

/// Both doors' checks, in the act's order of disclosure: the request's shape, then the resource's
/// existence, then the handle, through the act's own predicate. Each is answered before any
/// refusal could be recorded.
async fn check_request(pool: &PgPool, request: &FieldScrubRequest) -> ApiResult<()> {
    reject_malformed(request.field, request.family, request.clear)?;
    if erased_state(pool, request.resource).await?.is_none() {
        return Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string()));
    }
    let fault = sqlx::query_scalar!(
        r#"SELECT _resource_field_scrub_request_fault($1, $2, $3, $4) AS "fault""#,
        request.resource.uuid(),
        kind_name(request.field),
        request.family,
        request.clear,
    )
    .fetch_one(pool)
    .await?;
    match fault {
        Some(fault) => match classify_fault(&fault) {
            Some(fault) => Err(fault_message(fault, request.field, request.family)),
            // A fault this service does not know is drift between it and the act, not the
            // operator's error; its text is never rendered.
            None => Err(ApiError::Internal(
                "field scrub: the act's request check answered a fault this service does not know"
                    .to_string(),
            )),
        },
        None => Ok(()),
    }
}

/// The request's shape, checked without a read: a handle names a property family, so `property`
/// needs one and every other field takes none; `clear` names one field, so `properties` refuses it.
fn reject_malformed(field: ScrubFieldKind, family: Option<Uuid>, clear: bool) -> ApiResult<()> {
    let fault = match (field, family) {
        (ScrubFieldKind::Property, None) => Some(RequestFault::NeedsFamily),
        (ScrubFieldKind::Property, Some(_)) => None,
        (_, Some(_)) => Some(RequestFault::TakesNoFamily),
        (ScrubFieldKind::Properties, None) if clear => {
            Some(RequestFault::PropertiesCannotBeCleared)
        }
        (_, None) => None,
    };
    match fault {
        Some(fault) => Err(fault_message(fault, field, family)),
        None => Ok(()),
    }
}

/// The 400 for a request fault — one wording for the service's checks and the act's backstop.
/// Names only the field kind and the operator's own handle.
fn fault_message(fault: RequestFault, field: ScrubFieldKind, family: Option<Uuid>) -> ApiError {
    let kind = kind_name(field);
    match fault {
        RequestFault::UnknownFieldKind => ApiError::BadRequest(
            "unknown field kind; name title, origin_uri, property or properties; nothing was \
             scrubbed"
                .to_string(),
        ),
        RequestFault::NeedsFamily => ApiError::BadRequest(
            "field property needs a family handle (the family listing names each family's); \
             nothing was scrubbed"
                .to_string(),
        ),
        RequestFault::TakesNoFamily => ApiError::BadRequest(format!(
            "field {kind} takes no family handle; nothing was scrubbed"
        )),
        RequestFault::PropertiesCannotBeCleared => ApiError::BadRequest(
            "field properties cannot be cleared: clear names one field, a property family by its \
             handle; nothing was scrubbed"
                .to_string(),
        ),
        RequestFault::ForeignFamily(id) => listed_refusal(
            "family",
            id,
            family.as_slice(),
            "is not a property family handle of this resource (the family listing names each \
             family's); nothing was scrubbed",
        ),
    }
}

/// The 400 for a keep-mode request with nothing prior.
fn nothing_prior(field: ScrubFieldKind) -> ApiError {
    ApiError::BadRequest(format!(
        "field {} has nothing prior to scrub: no earlier value, or every one is already at its \
         sentinel; nothing was scrubbed",
        kind_name(field)
    ))
}

/// Run the act, retrying a deadlock up to `MAX_ACT_RETRIES` times (each retry is a fresh
/// statement and transaction), and map every other failure through the classifier.
async fn run_act(
    pool: &PgPool,
    attempt: &Attempt,
    request: &FieldScrubRequest,
) -> ApiResult<FieldScrubVerdict> {
    let mut retries = 0;
    loop {
        let result = sqlx::query_scalar!(
            r#"SELECT resource_field_scrub_execute($1, $2, $3, $4, $5, $6, $7)
                   AS "outcome: serde_json::Value""#,
            attempt.resource.uuid(),
            kind_name(request.field),
            request.family,
            request.clear,
            attempt.operator.uuid(),
            attempt.emitter.uuid(),
            attempt.request_reference,
        )
        .fetch_one(pool)
        .await;
        let err = match result {
            Ok(raw) => return decode_completion(raw).map(FieldScrubVerdict::Completed),
            Err(err) => err,
        };
        match classify_field_scrub_error(&err) {
            FieldScrubFailure::Retryable if retries < MAX_ACT_RETRIES => retries += 1,
            failure => return verdict_for(failure, request, err),
        }
    }
}

fn decode_completion(raw: Option<serde_json::Value>) -> ApiResult<ExecuteOutcomeWire> {
    let raw = raw.ok_or_else(|| {
        ApiError::Internal("resource_field_scrub_execute returned no row".to_string())
    })?;
    serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("field scrub outcome shape: {e}")))
}

/// What a classified failure means for the caller. Never carries the raise text: a refusal is
/// recorded vocabulary, a fault is a fixed message naming at most the field kind and the
/// operator's own handle, and anything unexpected is a scrubbed `Internal` (logged, not rendered).
fn verdict_for(
    failure: FieldScrubFailure,
    request: &FieldScrubRequest,
    err: sqlx::Error,
) -> ApiResult<FieldScrubVerdict> {
    match failure {
        FieldScrubFailure::Charter => Ok(FieldScrubVerdict::Refused(
            FieldScrubRefusalReason::CharterResource,
            Some(ResourceErasureRefusalDetail::MapGrainErasureTask),
        )),
        FieldScrubFailure::AlreadyErased => Ok(FieldScrubVerdict::Refused(
            FieldScrubRefusalReason::AlreadyErased,
            None,
        )),
        FieldScrubFailure::SentinelCollision => Ok(FieldScrubVerdict::Refused(
            FieldScrubRefusalReason::SentinelCollision,
            None,
        )),
        FieldScrubFailure::ProjectionDisagrees => Ok(FieldScrubVerdict::Refused(
            FieldScrubRefusalReason::ProjectionDisagrees,
            None,
        )),
        other => Err(failure_error(other, request, err, "field scrub failed")),
    }
}

/// The error for a failure that is not a recorded refusal, at either door.
fn failure_error(
    failure: FieldScrubFailure,
    request: &FieldScrubRequest,
    err: sqlx::Error,
    context: &str,
) -> ApiError {
    match failure {
        FieldScrubFailure::NotFound => ApiError::NotFound(RESOURCE_NOT_FOUND.to_string()),
        FieldScrubFailure::Fault(fault) => fault_message(fault, request.field, request.family),
        FieldScrubFailure::NothingPrior => nothing_prior(request.field),
        FieldScrubFailure::Charter
        | FieldScrubFailure::AlreadyErased
        | FieldScrubFailure::SentinelCollision
        | FieldScrubFailure::ProjectionDisagrees
        | FieldScrubFailure::Retryable
        | FieldScrubFailure::Other => ApiError::internal_scrubbed(context, err),
    }
}

/// Classify a failed act or plan call. A non-database error (a lost connection, a decode failure)
/// is `Other`.
fn classify_field_scrub_error(err: &sqlx::Error) -> FieldScrubFailure {
    match err.as_database_error() {
        Some(db) => classify_field_scrub_failure(db.code().as_deref(), db.message()),
        None => FieldScrubFailure::Other,
    }
}

/// The pure classifier over a database error's SQLSTATE and message. The literals it matches are
/// the act's and the plan's own raises (migration 20261018100020), all bare `RAISE EXCEPTION`
/// (SQLSTATE `P0001`); each `%` in them is an id or a field kind, so each arm matches a stable
/// prefix and suffix. A deadlock is matched by its SQLSTATE alone.
fn classify_field_scrub_failure(code: Option<&str>, message: &str) -> FieldScrubFailure {
    if code == Some(DEADLOCK_DETECTED) {
        return FieldScrubFailure::Retryable;
    }
    // Every message arm is a bare RAISE: the same text under any other SQLSTATE is not the act's.
    if code != Some(RAISE_EXCEPTION) {
        return FieldScrubFailure::Other;
    }
    match message
        .strip_prefix(EXECUTE_RAISE_PREFIX)
        .or_else(|| message.strip_prefix(PLAN_RAISE_PREFIX))
    {
        Some(rest) => classify_field_scrub_raise(rest),
        None => FieldScrubFailure::Other,
    }
}

/// The arms of `resource_field_scrub_execute`'s and `resource_field_scrub_plan`'s raises, after
/// the prefix. `p_resource is required`, `p_request_ref is required` and `p_clear is required` are
/// `Other`: the service always supplies all three, so each raise is a bug here, not a state of the
/// resource. So is `the plan refused (…)`, the act's defensive raise for a plan verdict it does not
/// know.
fn classify_field_scrub_raise(rest: &str) -> FieldScrubFailure {
    match rest {
        "already erased" => FieldScrubFailure::AlreadyErased,
        "sentinel collision" => FieldScrubFailure::SentinelCollision,
        "projection disagrees" => FieldScrubFailure::ProjectionDisagrees,
        "nothing prior to scrub" => FieldScrubFailure::NothingPrior,
        _ if rest.starts_with("charter resource") => FieldScrubFailure::Charter,
        _ if between(rest, "resource ", " not found").is_some() => FieldScrubFailure::NotFound,
        _ => match classify_fault(rest) {
            Some(fault) => FieldScrubFailure::Fault(fault),
            None => FieldScrubFailure::Other,
        },
    }
}

/// The request faults `_resource_field_scrub_request_fault` answers (migration 20261018100020,
/// section 1), which the act and the plan raise under their prefixes. `p_clear is required` is not
/// one: the service always supplies it.
fn classify_fault(fault: &str) -> Option<RequestFault> {
    if fault == "unknown field kind" {
        Some(RequestFault::UnknownFieldKind)
    } else if fault == "field property needs a family handle" {
        Some(RequestFault::NeedsFamily)
    } else if fault == "properties cannot be cleared" {
        Some(RequestFault::PropertiesCannotBeCleared)
    } else if between(fault, "field ", " takes no family handle").is_some() {
        Some(RequestFault::TakesNoFamily)
    } else if let Some((id, _)) = fault
        .strip_prefix("family ")
        .and_then(|r| r.split_once(" is not a property family handle of resource "))
    {
        Some(RequestFault::ForeignFamily(Uuid::parse_str(id).ok()))
    } else {
        None
    }
}

/// The read-only survey: what [`execute_field_scrub`] would do if it ran now, rendered from the
/// act's own plan (`resource_field_scrub_plan`, S5), beside the family listing.
///
/// The [`SystemAdmin`] proof is the gate, as for the act; the survey records nothing (a survey
/// attempt is not a scrub request). The request is checked exactly as the act checks it, in the
/// same order: a malformed request is a 400, an unknown resource is `NotFound`, a handle that is not
/// one of the resource's families is a 400 whatever the resource's state. Then a charter, an
/// erased resource, a sentinel collision or a projection that disagrees answers the refusal the act
/// would record, with no plan. In keep mode, a plan with no `redacted_fields` is the act's 400:
/// nothing prior.
pub async fn survey_field_scrub(
    pool: &PgPool,
    _admin: &SystemAdmin,
    request: FieldScrubRequest,
) -> ApiResult<FieldScrubSurvey> {
    check_request(pool, &request).await?;

    let raw = match sqlx::query_scalar!(
        r#"SELECT resource_field_scrub_survey($1, $2, $3, $4) AS "survey: serde_json::Value""#,
        request.resource.uuid(),
        kind_name(request.field),
        request.family,
        request.clear,
    )
    .fetch_one(pool)
    .await
    {
        Ok(raw) => raw,
        Err(err) => {
            let failure = classify_field_scrub_error(&err);
            return Err(failure_error(
                failure,
                &request,
                err,
                "field scrub survey failed",
            ));
        }
    };
    let raw = raw.ok_or_else(|| {
        ApiError::Internal("resource_field_scrub_survey returned no row".to_string())
    })?;
    let wire: SurveyWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("field scrub survey shape: {e}")))?;

    let families = match wire.families {
        Some(rows) => Some(with_flags(pool, request.resource, rows).await?),
        None => None,
    };
    let refused = |reason, detail: Option<ResourceErasureRefusalDetail>| FieldScrubSurvey {
        resource: request.resource,
        refusal: Some(reason),
        detail: detail.map(|d| d.as_str().to_string()),
        families: families.clone(),
        plan: None,
    };
    match wire.plan.refusal.as_deref() {
        Some("charter_resource") => {
            return Ok(refused(
                FieldScrubRefusalReason::CharterResource,
                Some(ResourceErasureRefusalDetail::MapGrainErasureTask),
            ))
        }
        Some("already_erased") => return Ok(refused(FieldScrubRefusalReason::AlreadyErased, None)),
        Some("sentinel_collision") => {
            return Ok(refused(FieldScrubRefusalReason::SentinelCollision, None))
        }
        Some("projection_disagrees") => {
            return Ok(refused(FieldScrubRefusalReason::ProjectionDisagrees, None))
        }
        // Keep mode with nothing prior: the plan says so with an empty `redacted_fields`, and the
        // act answers it 400. Never under `clear`, whose clearing event makes the field prior.
        Some("nothing_prior") | None => {}
        Some(_) => {
            return Err(ApiError::Internal(
                "field scrub survey: the plan named a refusal this service does not know"
                    .to_string(),
            ))
        }
    }

    Ok(FieldScrubSurvey {
        resource: request.resource,
        refusal: None,
        detail: None,
        families,
        plan: Some(FieldScrubPlan {
            redacted_fields: wire.plan.redacted_fields,
            kept: wire.plan.kept,
            unreachable: wire.plan.unreachable,
            folded_rows: wire.plan.folded_rows,
            clears: wire.plan.clears,
        }),
    })
}

/// The family listing (S1): the title, the origin URI and each resource-owned property family by
/// its handle, with the sweep's flags. Structure only. Records nothing. An unknown resource is
/// `NotFound`.
pub async fn list_field_scrub_families(
    pool: &PgPool,
    _admin: &SystemAdmin,
    resource: ResourceId,
) -> ApiResult<FieldScrubFamilies> {
    if erased_state(pool, resource).await?.is_none() {
        return Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string()));
    }
    let rows = sqlx::query_scalar!(
        r#"SELECT to_jsonb(f) - 'n' AS "row!: serde_json::Value"
             FROM resource_field_scrub_families($1) WITH ORDINALITY AS f(field, family, events, live,
                  unset, first_seen, first_by, value_type, n)
            ORDER BY f.n"#,
        resource.uuid(),
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(serde_json::from_value)
    .collect::<Result<Vec<FieldScrubFamily>, _>>()
    .map_err(|e| ApiError::Internal(format!("field scrub family shape: {e}")))?;
    Ok(FieldScrubFamilies {
        resource,
        families: with_flags(pool, resource, rows).await?,
    })
}

/// Set each listed family's sweep flags from `resource_field_scrub_family_flags`, which answers one
/// row per row of the listing, keyed as the listing keys it (`field`, `family`). A family the flags
/// read does not answer is an `Internal` error, never an unflagged row: an absent flag would read
/// as "the sweep found nothing".
async fn with_flags(
    pool: &PgPool,
    resource: ResourceId,
    mut families: Vec<FieldScrubFamily>,
) -> ApiResult<Vec<FieldScrubFamily>> {
    let flags = sqlx::query!(
        r#"SELECT field AS "field!", family, flagged AS "flagged!",
                  current_flagged AS "current_flagged!", covered AS "covered!"
             FROM resource_field_scrub_family_flags($1)"#,
        resource.uuid(),
    )
    .fetch_all(pool)
    .await?;
    for family in &mut families {
        let row = flags
            .iter()
            .find(|f| {
                f.field == kind_name(family.field) && f.family == family.family.map(EventId::uuid)
            })
            .ok_or_else(|| {
                ApiError::Internal(
                    "field scrub family flags: a listed family has no flags row".to_string(),
                )
            })?;
        family.flagged = row.flagged;
        family.current_flagged = row.current_flagged;
        family.covered = row.covered;
    }
    Ok(families)
}

#[cfg(test)]
mod classifier_tests {
    //! The classifier over every literal the act and the plan raise (migration 20261018100020),
    //! `%` substituted with an id or a kind. Each FAILS IF the classifier's arm for that literal
    //! drifts.
    use super::*;

    const P0001: Option<&str> = Some("P0001");

    fn id() -> Uuid {
        Uuid::parse_str("01a0e9e7-491d-7700-8f58-99d0b068e059").expect("a literal uuid")
    }

    fn other_id() -> Uuid {
        Uuid::parse_str("01a0e9e7-491d-7700-8f58-99d0b068e05a").expect("a literal uuid")
    }

    fn act(rest: &str) -> FieldScrubFailure {
        classify_field_scrub_failure(P0001, &format!("{EXECUTE_RAISE_PREFIX}{rest}"))
    }

    #[test]
    fn the_required_argument_raises_are_other() {
        for rest in [
            "p_resource is required",
            "p_request_ref is required",
            "p_clear is required",
        ] {
            assert_eq!(act(rest), FieldScrubFailure::Other, "{rest}");
        }
    }

    #[test]
    fn the_defensive_plan_refused_raise_is_other() {
        assert_eq!(
            act("the plan refused (something_new)"),
            FieldScrubFailure::Other
        );
    }

    #[test]
    fn resource_not_found_is_not_found_at_the_act_and_the_plan() {
        assert_eq!(
            act(&format!("resource {} not found", id())),
            FieldScrubFailure::NotFound
        );
        assert_eq!(
            classify_field_scrub_failure(
                P0001,
                &format!("resource_field_scrub_plan: resource {} not found", id())
            ),
            FieldScrubFailure::NotFound
        );
    }

    #[test]
    fn the_recorded_refusals_classify_to_their_reasons() {
        assert_eq!(
            act("charter resource (map-grain erasure is filed task \
                 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)"),
            FieldScrubFailure::Charter
        );
        assert_eq!(act("already erased"), FieldScrubFailure::AlreadyErased);
        assert_eq!(
            act("sentinel collision"),
            FieldScrubFailure::SentinelCollision
        );
        assert_eq!(
            act("projection disagrees"),
            FieldScrubFailure::ProjectionDisagrees
        );
    }

    #[test]
    fn nothing_prior_is_its_own_400() {
        assert_eq!(
            act("nothing prior to scrub"),
            FieldScrubFailure::NothingPrior
        );
    }

    #[test]
    fn every_request_fault_classifies() {
        assert_eq!(
            act("unknown field kind"),
            FieldScrubFailure::Fault(RequestFault::UnknownFieldKind)
        );
        assert_eq!(
            act("field property needs a family handle"),
            FieldScrubFailure::Fault(RequestFault::NeedsFamily)
        );
        assert_eq!(
            act("field origin_uri takes no family handle"),
            FieldScrubFailure::Fault(RequestFault::TakesNoFamily)
        );
        assert_eq!(
            act("properties cannot be cleared"),
            FieldScrubFailure::Fault(RequestFault::PropertiesCannotBeCleared)
        );
        assert_eq!(
            classify_field_scrub_failure(
                P0001,
                "resource_field_scrub_plan: properties cannot be cleared"
            ),
            FieldScrubFailure::Fault(RequestFault::PropertiesCannotBeCleared)
        );
    }

    // FAILS IF the foreign-family arm loses the handle, or reads the RESOURCE id as the handle.
    #[test]
    fn a_foreign_family_names_the_handle_not_the_resource() {
        assert_eq!(
            act(&format!(
                "family {} is not a property family handle of resource {}",
                id(),
                other_id()
            )),
            FieldScrubFailure::Fault(RequestFault::ForeignFamily(Some(id())))
        );
    }

    #[test]
    fn a_deadlock_is_retryable_whatever_its_message() {
        assert_eq!(
            classify_field_scrub_failure(Some("40P01"), "deadlock detected"),
            FieldScrubFailure::Retryable
        );
    }

    // FAILS IF a message arm matches the act's text under a SQLSTATE that is not a bare RAISE.
    #[test]
    fn the_right_message_under_the_wrong_code_is_other() {
        let msg = "resource_field_scrub_execute: already erased";
        assert_eq!(
            classify_field_scrub_failure(Some("XX000"), msg),
            FieldScrubFailure::Other
        );
        assert_eq!(
            classify_field_scrub_failure(None, msg),
            FieldScrubFailure::Other
        );
    }

    // FAILS IF the field scrub's classifier reads another act's raises or an unknown raise as
    // anything but `Other`.
    #[test]
    fn an_unrelated_raise_is_other() {
        for msg in [
            "block_history_scrub_execute: already erased",
            "resource_erasure_execute: already erased",
            "resource_field_scrub_execute: something new",
        ] {
            assert_eq!(
                classify_field_scrub_failure(P0001, msg),
                FieldScrubFailure::Other,
                "{msg}"
            );
        }
    }

    // FAILS IF a malformed shape passes, or a well-formed one is refused.
    #[test]
    fn the_shape_is_checked_without_a_read() {
        use ScrubFieldKind as K;
        let bad = [
            (K::Property, None, false),
            (K::Title, Some(id()), false),
            (K::OriginUri, Some(id()), true),
            (K::Properties, Some(id()), false),
            (K::Properties, None, true),
        ];
        for (field, family, clear) in bad {
            assert!(
                matches!(
                    reject_malformed(field, family, clear),
                    Err(ApiError::BadRequest(_))
                ),
                "{field:?} {family:?} clear={clear}"
            );
        }
        let good = [
            (K::Property, Some(id()), false),
            (K::Property, Some(id()), true),
            (K::Title, None, true),
            (K::OriginUri, None, false),
            (K::Properties, None, false),
        ];
        for (field, family, clear) in good {
            assert!(
                reject_malformed(field, family, clear).is_ok(),
                "{field:?} {family:?} clear={clear}"
            );
        }
    }

    // FAILS IF a foreign-family 400 names an id the operator did not send, or stops naming the
    // one the operator did.
    #[test]
    fn a_foreign_family_400_names_only_the_operators_own_handle() {
        match fault_message(
            RequestFault::ForeignFamily(Some(id())),
            ScrubFieldKind::Property,
            Some(id()),
        ) {
            ApiError::BadRequest(msg) => assert!(msg.contains(&id().to_string()), "{msg}"),
            other => panic!("a foreign family is a 400, got {other:?}"),
        }
        match fault_message(
            RequestFault::ForeignFamily(Some(other_id())),
            ScrubFieldKind::Property,
            Some(id()),
        ) {
            ApiError::BadRequest(msg) => {
                assert!(!msg.contains(&other_id().to_string()), "{msg}");
                assert!(msg.starts_with("a listed family"), "{msg}");
            }
            other => panic!("a foreign family is a 400, got {other:?}"),
        }
    }
}
