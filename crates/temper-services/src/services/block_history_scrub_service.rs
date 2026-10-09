//! The block history scrub's service layer — execute + survey (resource erasure spec 2026-09-28,
//! build order 2e; D11, D5, D10). The resource erasure's service (`resource_erasure_service`) is
//! the template and its posture holds here unchanged: the gate is the sealed [`SystemAdmin`]
//! proof and nothing more, the request reference is minted here (one `Uuid::now_v7()` per act),
//! the act's raises are mapped through a closed classifier (`classify_scrub_failure`), and
//! nothing the SQL says reaches a response. The helpers the two acts share (the `Attempt`,
//! the retry bound, the SQLSTATEs, the refusal recorder, the list checks) live in that module
//! and are shared by visibility, never copied.
//!
//! SQL commits, it does not decide legality: the plan, the refusals and the one redaction body
//! narrowed to a block set live in `block_history_scrub_plan` / `block_history_scrub_survey` /
//! `block_history_scrub_execute` (migration 20261003000210). A malformed list (empty, repeated,
//! or naming an id that is not a block of the resource) is a 400 that records nothing and empties
//! nothing, whatever state the resource is in: membership is checked against `kb_content_blocks`
//! before any refusal is recorded, so a refusal names only real blocks of the resource, never
//! more than it has. A refusal the act raises (a charter, an erased resource) is then recorded
//! through `resource_erasure_refuse` with the scrub's `act` and those blocks.
//!
//! The survey records nothing. The SQL survey has no refusal verdicts and silently skips an id
//! that is not a block of the resource, so the survey checks both here, in the execute door's
//! order: a foreign id is the same 400 the act answers, and then an erased or charter resource
//! answers the refusal the act would record. The HTTP doors call straight into [`execute_block_history_scrub`] /
//! [`survey_block_history_scrub`]; this module carries no HTTP types.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::{
    BlockHistoryScrubPlan, BlockHistoryScrubSurvey, BlockScrubCount,
};
use temper_core::types::ids::ResourceId;
use temper_substrate::payloads::{ErasureAct, ErasureTargetOutcome, ResourceErasureRefusalReason};
use temper_substrate::writes::resolve_emitter;
use temper_workflow::operations::Surface;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};
use crate::services::resource_erasure_service::{
    between, erased_state, listed_refusal, refuse, reject_duplicates, Attempt,
    ResourceErasureRefusal, ResourceErasureRefusalDetail, DEADLOCK_DETECTED, MAX_ACT_RETRIES,
    RAISE_EXCEPTION, RESOURCE_NOT_FOUND,
};

/// The prefix of every raise in `block_history_scrub_execute` (migration 20261003000210).
const EXECUTE_RAISE_PREFIX: &str = "block_history_scrub_execute: ";

/// One execute request. The operator is not here (the [`SystemAdmin`] proof names it), and
/// neither is the request reference: the service mints it.
#[derive(Debug, Clone, Copy)]
pub struct BlockHistoryScrubRequest<'a> {
    pub resource: ResourceId,
    /// The blocks whose history is scrubbed: each must be a block of `resource`, named once.
    pub blocks: &'a [Uuid],
    /// Where the request came from; the act and any refusal are attributed through it.
    pub surface: Surface,
}

/// A completed scrub: ONE `block_history_scrubbed` event stands behind these values.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockHistoryScrubCompletion {
    /// The server-minted reference the act is correlated by — the operator cites it.
    pub request_reference: Uuid,
    pub event_id: Uuid,
    /// One line per named block, in the operator's order, then the ingest line when the scrub
    /// cancelled an in-flight ingest.
    pub targets: Vec<ErasureTargetOutcome>,
    /// True when the scrub cancelled an in-flight ingest (ruling 3 of 2e).
    pub cancelled_ingest: bool,
}

/// What an execute call did.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockHistoryScrubOutcome {
    Completed(BlockHistoryScrubCompletion),
    Refused(ResourceErasureRefusal),
}

/// The jsonb `block_history_scrub_execute` returns.
#[derive(Debug, serde::Deserialize)]
struct ExecuteOutcomeWire {
    event_id: Uuid,
    targets: Vec<ErasureTargetOutcome>,
}

/// One committed run of the act: its return, and the recorded payload's `cancelled_ingest`.
#[derive(Debug)]
struct ScrubCommitted {
    wire: ExecuteOutcomeWire,
    cancelled_ingest: bool,
}

/// The jsonb `block_history_scrub_survey` returns (its `resource` and `ingest_state` keys are
/// ignored: the caller named the resource, and the raw ingest state is not a wire value).
#[derive(Debug, serde::Deserialize)]
struct SurveyPlanWire {
    cancels_ingest: bool,
    blocks: Vec<BlockScrubCount>,
}

/// How one run of the act ended, once the classifier has read any failure.
#[derive(Debug)]
enum ScrubVerdict {
    Completed(ScrubCommitted),
    Refused(
        ResourceErasureRefusalReason,
        Option<ResourceErasureRefusalDetail>,
    ),
}

/// The closed classification of a failed `block_history_scrub_execute` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrubFailure {
    Charter,
    AlreadyErased,
    NotFound,
    EmptyBlocks,
    /// A listed id that is not a block of the resource; the id when the message carried a
    /// parseable one.
    ForeignBlock(Option<Uuid>),
    /// A listed block named twice (the service refuses this first; the raise is the backstop).
    RepeatedBlock(Option<Uuid>),
    /// A deadlock.
    Retryable,
    Other,
}

/// Execute the block history scrub, as the operator `admin` names.
///
/// The [`SystemAdmin`] proof is the gate, and it ran where the proof was minted. The operator's
/// emitter resolves first. An empty list, or one naming a block twice, is
/// [`ApiError::BadRequest`] before the act runs; an unknown resource is [`ApiError::NotFound`].
/// A listed id that is not a block of the resource is [`ApiError::BadRequest`], for every state
/// of the resource, checked before the act runs and before any refusal is recorded: nothing was
/// scrubbed and nothing was recorded. A charter or an already-erased resource is then a recorded
/// refusal naming the scrub and its blocks, each a real block of the resource. The act checks
/// membership again under its lock, as the backstop.
pub async fn execute_block_history_scrub(
    pool: &PgPool,
    admin: &SystemAdmin,
    request: BlockHistoryScrubRequest<'_>,
) -> ApiResult<BlockHistoryScrubOutcome> {
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
    reject_malformed_list(request.blocks)?;
    if erased_state(pool, request.resource).await?.is_none() {
        return Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string()));
    }
    // Membership before the act, so a refusal (recorded in a ledger nothing redacts) only ever
    // names real blocks of the resource.
    if let Some(foreign) = first_foreign_block(pool, request.resource, request.blocks).await? {
        return Err(foreign_block(Some(foreign), request.blocks));
    }

    let committed = match run_scrub(pool, &attempt, request.blocks).await? {
        ScrubVerdict::Completed(committed) => committed,
        ScrubVerdict::Refused(reason, detail) => {
            let refusal = refuse(
                pool,
                &attempt,
                ErasureAct::BlockHistoryScrub,
                request.blocks,
                reason,
                detail,
            )
            .await?;
            return Ok(BlockHistoryScrubOutcome::Refused(refusal));
        }
    };

    Ok(BlockHistoryScrubOutcome::Completed(
        BlockHistoryScrubCompletion {
            request_reference: attempt.request_reference,
            event_id: committed.wire.event_id,
            targets: committed.wire.targets,
            cancelled_ingest: committed.cancelled_ingest,
        },
    ))
}

/// An empty list scrubs nothing, and a list naming one block twice is a false account of the
/// request: both are refused before the act touches the resource, at both doors.
fn reject_malformed_list(blocks: &[Uuid]) -> ApiResult<()> {
    if blocks.is_empty() {
        return Err(empty_list());
    }
    reject_duplicates(blocks, "block", "scrubbed")
}

/// The 400 for an empty list — one wording for the service's check and the act's backstop.
fn empty_list() -> ApiError {
    ApiError::BadRequest(
        "blocks is empty; name at least one block of the resource; nothing was scrubbed"
            .to_string(),
    )
}

/// Run the act, retrying a deadlock up to `MAX_ACT_RETRIES` times (each retry is a fresh
/// transaction), and map every other failure through the classifier.
async fn run_scrub(pool: &PgPool, attempt: &Attempt, blocks: &[Uuid]) -> ApiResult<ScrubVerdict> {
    let mut retries = 0;
    loop {
        let err = match scrub_once(pool, attempt, blocks).await {
            Ok(committed) => return Ok(ScrubVerdict::Completed(committed)),
            Err(err) => err,
        };
        match classify_scrub_error(&err) {
            ScrubFailure::Retryable if retries < MAX_ACT_RETRIES => retries += 1,
            failure => return verdict_for(failure, blocks, err),
        }
    }
}

/// ONE transaction: the act, then its recorded payload's `cancelled_ingest`, then the commit.
/// The structured field is read from the ledger, never inferred from a `targets` line (ruling 6
/// of 2e: the field is authoritative, the line is for readers). Reading it before the commit
/// means any failure here rolls the act back whole, so nothing that committed answers as a
/// failure. A malformed return is a decode error, which the classifier reads as `Other`.
async fn scrub_once(
    pool: &PgPool,
    attempt: &Attempt,
    blocks: &[Uuid],
) -> Result<ScrubCommitted, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let raw = sqlx::query_scalar!(
        r#"SELECT block_history_scrub_execute($1, $2, $3, $4, $5)
               AS "outcome: serde_json::Value""#,
        attempt.resource.uuid(),
        blocks,
        attempt.operator.uuid(),
        attempt.emitter.uuid(),
        attempt.request_reference,
    )
    .fetch_one(&mut *tx)
    .await?
    .ok_or_else(|| sqlx::Error::Decode("block_history_scrub_execute returned no row".into()))?;
    let wire: ExecuteOutcomeWire =
        serde_json::from_value(raw).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let cancelled_ingest = sqlx::query_scalar!(
        r#"SELECT COALESCE((payload->>'cancelled_ingest')::boolean, false) AS "cancelled!"
             FROM kb_events
            WHERE id = $1"#,
        wire.event_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ScrubCommitted {
        wire,
        cancelled_ingest,
    })
}

/// What a classified failure means for the caller. Never carries the raise text: a refusal is
/// recorded vocabulary, a list refusal is a fixed message naming at most the operator's own
/// block id, and anything unexpected is a scrubbed `Internal` (logged, not rendered).
fn verdict_for(
    failure: ScrubFailure,
    supplied: &[Uuid],
    err: sqlx::Error,
) -> ApiResult<ScrubVerdict> {
    match failure {
        ScrubFailure::Charter => Ok(ScrubVerdict::Refused(
            ResourceErasureRefusalReason::CharterResource,
            Some(ResourceErasureRefusalDetail::MapGrainErasureTask),
        )),
        ScrubFailure::AlreadyErased => Ok(ScrubVerdict::Refused(
            ResourceErasureRefusalReason::AlreadyErased,
            None,
        )),
        ScrubFailure::NotFound => Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string())),
        ScrubFailure::EmptyBlocks => Err(empty_list()),
        ScrubFailure::ForeignBlock(block) => Err(foreign_block(block, supplied)),
        ScrubFailure::RepeatedBlock(block) => Err(listed_refusal(
            "block",
            block,
            supplied,
            "is listed more than once; list each block once; nothing was scrubbed",
        )),
        ScrubFailure::Retryable | ScrubFailure::Other => Err(ApiError::internal_scrubbed(
            "block history scrub failed",
            err,
        )),
    }
}

/// The 400 for a listed id that is not a block of the resource — one wording at both doors.
fn foreign_block(block: Option<Uuid>, supplied: &[Uuid]) -> ApiError {
    listed_refusal(
        "block",
        block,
        supplied,
        "is not a block of this resource; nothing was scrubbed",
    )
}

/// Classify a failed act call. A non-database error (a lost connection, a decode failure) is
/// `Other`.
fn classify_scrub_error(err: &sqlx::Error) -> ScrubFailure {
    match err.as_database_error() {
        Some(db) => classify_scrub_failure(db.code().as_deref(), db.message()),
        None => ScrubFailure::Other,
    }
}

/// The pure classifier over a database error's SQLSTATE and message. The literals it matches
/// are the act's own raises (migration 20261003000210), all bare `RAISE EXCEPTION` (SQLSTATE
/// `P0001`); each `%` in them is an id, so each arm matches a stable prefix and suffix. A
/// deadlock is matched by its SQLSTATE alone.
fn classify_scrub_failure(code: Option<&str>, message: &str) -> ScrubFailure {
    if code == Some(DEADLOCK_DETECTED) {
        return ScrubFailure::Retryable;
    }
    // Every message arm is a bare RAISE: the same text under any other SQLSTATE is not the act's.
    if code != Some(RAISE_EXCEPTION) {
        return ScrubFailure::Other;
    }
    match message.strip_prefix(EXECUTE_RAISE_PREFIX) {
        Some(rest) => classify_scrub_raise(rest),
        None => ScrubFailure::Other,
    }
}

/// The arms of `block_history_scrub_execute`'s raises, after its prefix.
/// `p_resource is required` and `p_request_ref is required` are `Other`: the service always
/// supplies both, so either raise is a bug here, not a state of the resource.
fn classify_scrub_raise(rest: &str) -> ScrubFailure {
    if rest == "already erased" {
        ScrubFailure::AlreadyErased
    } else if rest.starts_with("charter resource") {
        ScrubFailure::Charter
    } else if rest == "p_blocks is empty" {
        ScrubFailure::EmptyBlocks
    } else if let Some((id, _)) = rest
        .strip_prefix("block ")
        .and_then(|r| r.split_once(" is not a block of resource "))
    {
        ScrubFailure::ForeignBlock(Uuid::parse_str(id).ok())
    } else if let Some(id) = between(rest, "block ", " is named more than once") {
        ScrubFailure::RepeatedBlock(Uuid::parse_str(id).ok())
    } else if between(rest, "resource ", " not found").is_some() {
        ScrubFailure::NotFound
    } else {
        ScrubFailure::Other
    }
}

/// The read-only survey: what [`execute_block_history_scrub`] would do if it ran now, rendered
/// from the act's own plan (`block_history_scrub_plan`, D10).
///
/// The [`SystemAdmin`] proof is the gate, as for the act; the survey records nothing (a survey
/// attempt is not a scrub request). The list is checked exactly as the act checks it, in the
/// same order: empty or repeated is a 400, an unknown resource is `NotFound`, an id that is not a
/// block of the resource is a 400 whatever the resource's state, and then a charter or an erased
/// resource answers the refusal the act would record (with no per-block rows).
///
/// Each block also says whether its current revision still holds an open finding of the
/// sensitivity sweep (D11's warning: the text has not been edited out yet). It is read after the
/// plan, through `block_history_scrub_flagged_blocks`, and the act never reads it.
pub async fn survey_block_history_scrub(
    pool: &PgPool,
    _admin: &SystemAdmin,
    resource: ResourceId,
    blocks: &[Uuid],
) -> ApiResult<BlockHistoryScrubSurvey> {
    reject_malformed_list(blocks)?;
    let Some(state) = scrub_state(pool, resource).await? else {
        return Err(ApiError::NotFound(RESOURCE_NOT_FOUND.to_string()));
    };
    if let Some(foreign) = first_foreign_block(pool, resource, blocks).await? {
        return Err(foreign_block(Some(foreign), blocks));
    }

    // The act's own order: a charter refuses before an erased resource does.
    let refused = |reason, detail: Option<ResourceErasureRefusalDetail>| BlockHistoryScrubSurvey {
        resource,
        refusal: Some(reason),
        detail: detail.map(|d| d.as_str().to_string()),
        plan: None,
    };
    if state.charter {
        return Ok(refused(
            ResourceErasureRefusalReason::CharterResource,
            Some(ResourceErasureRefusalDetail::MapGrainErasureTask),
        ));
    }
    if state.erased {
        return Ok(refused(ResourceErasureRefusalReason::AlreadyErased, None));
    }

    let raw = sqlx::query_scalar!(
        r#"SELECT block_history_scrub_survey($1, $2) AS "survey: serde_json::Value""#,
        resource.uuid(),
        blocks,
    )
    .fetch_one(pool)
    .await?
    .ok_or_else(|| ApiError::Internal("block_history_scrub_survey returned no row".to_string()))?;
    let mut wire: SurveyPlanWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("block history scrub survey shape: {e}")))?;
    let flagged = sqlx::query_scalar!(
        r#"SELECT block_history_scrub_flagged_blocks($1, $2) AS "flagged!: Vec<Uuid>""#,
        resource.uuid(),
        blocks,
    )
    .fetch_one(pool)
    .await?;
    for block in &mut wire.blocks {
        block.current_revision_flagged = flagged.contains(&block.block);
    }

    Ok(BlockHistoryScrubSurvey {
        resource,
        refusal: None,
        detail: None,
        plan: Some(BlockHistoryScrubPlan {
            cancels_ingest: wire.cancels_ingest,
            blocks: wire.blocks,
        }),
    })
}

/// The two refusal verdicts the act reads under its lock, read here without one (a survey).
#[derive(Debug, Clone, Copy)]
struct ScrubState {
    erased: bool,
    charter: bool,
}

/// `None` when no such resource exists; otherwise the act's two refusal verdicts. A charter is
/// a resource some cogmap names as its telos — the act's own predicate.
async fn scrub_state(pool: &PgPool, resource: ResourceId) -> ApiResult<Option<ScrubState>> {
    let row = sqlx::query!(
        r#"SELECT r.erased_at IS NOT NULL AS "erased!",
                  EXISTS (SELECT 1 FROM kb_cogmaps c WHERE c.telos_resource_id = r.id)
                      AS "charter!"
             FROM kb_resources r
            WHERE r.id = $1"#,
        resource.uuid(),
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| ScrubState {
        erased: r.erased,
        charter: r.charter,
    }))
}

/// The first listed id, in the operator's order, that is not a block of `resource` — the act's
/// own membership predicate. Both doors check it before anything else reads the resource's state:
/// the SQL survey skips such ids, and the act raises them only after its refusals.
async fn first_foreign_block(
    pool: &PgPool,
    resource: ResourceId,
    blocks: &[Uuid],
) -> ApiResult<Option<Uuid>> {
    let foreign = sqlx::query_scalar!(
        r#"SELECT x.id AS "id!"
             FROM unnest($2::uuid[]) WITH ORDINALITY AS x(id, n)
            WHERE NOT EXISTS (SELECT 1 FROM kb_content_blocks b
                               WHERE b.id = x.id AND b.resource_id = $1)
            ORDER BY x.n
            LIMIT 1"#,
        resource.uuid(),
        blocks,
    )
    .fetch_optional(pool)
    .await?;
    Ok(foreign)
}

#[cfg(test)]
mod classifier_tests {
    //! The classifier over every literal the act raises (migration 20261003000210), `%`
    //! substituted with an id. Each FAILS IF the classifier's arm for that literal drifts.
    use super::*;

    const P0001: Option<&str> = Some("P0001");

    fn id() -> Uuid {
        Uuid::parse_str("01a0e9e7-491d-7700-8f58-99d0b068e059").expect("a literal uuid")
    }

    fn other_id() -> Uuid {
        Uuid::parse_str("01a0e9e7-491d-7700-8f58-99d0b068e05a").expect("a literal uuid")
    }

    #[test]
    fn the_required_argument_raises_are_other() {
        assert_eq!(
            classify_scrub_failure(P0001, "block_history_scrub_execute: p_resource is required"),
            ScrubFailure::Other
        );
        assert_eq!(
            classify_scrub_failure(
                P0001,
                "block_history_scrub_execute: p_request_ref is required"
            ),
            ScrubFailure::Other
        );
    }

    #[test]
    fn resource_not_found_is_not_found() {
        let msg = format!("block_history_scrub_execute: resource {} not found", id());
        assert_eq!(classify_scrub_failure(P0001, &msg), ScrubFailure::NotFound);
    }

    #[test]
    fn the_charter_raise_is_charter() {
        assert_eq!(
            classify_scrub_failure(
                P0001,
                "block_history_scrub_execute: charter resource (map-grain erasure is filed task \
                 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)"
            ),
            ScrubFailure::Charter
        );
    }

    #[test]
    fn already_erased_is_already_erased() {
        assert_eq!(
            classify_scrub_failure(P0001, "block_history_scrub_execute: already erased"),
            ScrubFailure::AlreadyErased
        );
    }

    #[test]
    fn an_empty_list_is_empty_blocks() {
        assert_eq!(
            classify_scrub_failure(P0001, "block_history_scrub_execute: p_blocks is empty"),
            ScrubFailure::EmptyBlocks
        );
    }

    // FAILS IF the foreign-block arm loses the block id, or reads the RESOURCE id as the block.
    #[test]
    fn a_foreign_block_names_the_block_not_the_resource() {
        let msg = format!(
            "block_history_scrub_execute: block {} is not a block of resource {}",
            id(),
            other_id()
        );
        assert_eq!(
            classify_scrub_failure(P0001, &msg),
            ScrubFailure::ForeignBlock(Some(id()))
        );
    }

    #[test]
    fn a_repeated_block_names_its_id() {
        let msg = format!(
            "block_history_scrub_execute: block {} is named more than once",
            id()
        );
        assert_eq!(
            classify_scrub_failure(P0001, &msg),
            ScrubFailure::RepeatedBlock(Some(id()))
        );
    }

    #[test]
    fn a_deadlock_is_retryable_whatever_its_message() {
        assert_eq!(
            classify_scrub_failure(Some("40P01"), "deadlock detected"),
            ScrubFailure::Retryable
        );
    }

    // FAILS IF a message arm matches the act's text under a SQLSTATE that is not a bare RAISE.
    #[test]
    fn the_right_message_under_the_wrong_code_is_other() {
        let msg = "block_history_scrub_execute: already erased";
        assert_eq!(
            classify_scrub_failure(P0001, msg),
            ScrubFailure::AlreadyErased
        );
        assert_eq!(
            classify_scrub_failure(Some("XX000"), msg),
            ScrubFailure::Other
        );
        assert_eq!(classify_scrub_failure(None, msg), ScrubFailure::Other);
    }

    // FAILS IF the scrub's classifier reads the erasure act's raises (another act's prefix) or
    // an unknown raise as anything but `Other`.
    #[test]
    fn an_unrelated_raise_is_other() {
        assert_eq!(
            classify_scrub_failure(P0001, "resource_erasure_execute: already erased"),
            ScrubFailure::Other
        );
        assert_eq!(
            classify_scrub_failure(P0001, "block_history_scrub_execute: something new"),
            ScrubFailure::Other
        );
    }

    // FAILS IF a foreign-block 400 names an id the operator did not list, or stops naming one
    // the operator did.
    #[test]
    fn a_foreign_block_400_names_only_the_operators_own_id() {
        match foreign_block(Some(id()), &[id()]) {
            ApiError::BadRequest(msg) => assert!(msg.contains(&id().to_string()), "{msg}"),
            other => panic!("a foreign block is a 400, got {other:?}"),
        }
        match foreign_block(Some(other_id()), &[id()]) {
            ApiError::BadRequest(msg) => {
                assert!(!msg.contains(&other_id().to_string()), "{msg}");
                assert!(msg.starts_with("a listed block"), "{msg}");
            }
            other => panic!("a foreign block is a 400, got {other:?}"),
        }
    }

    // FAILS IF an empty or repeated list passes, or the repeat's 400 does not name the block.
    #[test]
    fn an_empty_or_repeated_list_is_a_400() {
        assert!(matches!(
            reject_malformed_list(&[]),
            Err(ApiError::BadRequest(_))
        ));
        assert!(reject_malformed_list(&[id(), other_id()]).is_ok());
        match reject_malformed_list(&[id(), other_id(), id()]) {
            Err(ApiError::BadRequest(msg)) => {
                assert!(msg.contains(&id().to_string()), "{msg}");
                assert!(!msg.contains(&other_id().to_string()), "{msg}");
                assert!(msg.ends_with("nothing was scrubbed"), "{msg}");
            }
            other => panic!("a repeated block is a 400, got {other:?}"),
        }
    }
}
