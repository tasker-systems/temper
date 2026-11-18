//! The erasure act's service layer — execute + survey (spec 2026-08-31, task 01a0577c Beat 2).
//!
//! THE DOOR IS OPERATOR-ONLY (ruled 2026-09-09): the instance-level operator — the
//! `is_system_admin` standing — is the ONLY executor; self-serve is a non-goal of this build.
//! Both functions take the sealed [`SystemAdmin`] proof, so the gate is the signature: a caller
//! who is not a system admin cannot reach them, and the surface that mints the proof rejects that
//! caller before dispatch with no ledger event (`handlers::erasure`, ruled 2026-09-30). The act is
//! attributed to `admin.actor()`.
//!
//! SQL commits, it does not decide legality (`principal_standing_apply`'s shape,
//! migrations/20260720000030): the scope computation, the per-row governed-home strikes, the
//! tombstone machinery and the completion event live in `principal_erasure_execute` /
//! `_erasure_apply_redaction` (migration 20260909000025; the computation itself moved whole into
//! `principal_erasure_survey_plan`, 20260913000010, which the act consumes and the survey door
//! serves). The admin surface's HTTP doors (`handlers::erasure::execute`,
//! `handlers::erasure::survey`) call straight into [`execute_erasure`] / [`survey_erasure`] —
//! this module stays the service layer and carries no HTTP types.
//!
//! The request reference is an opaque UUID supplied by the caller: it rides
//! `kb_events."references"` (rel `request`) and the act's correlation id — never the payload —
//! so the request-to-person mapping stays in the operator's DSAR records, outside the ledger.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::erasure::{
    EstateCounts, EstateErasureKind, EstateResourceErasure, EstateResourcePlan,
    SlackRevocationTally,
};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_substrate::payloads::ErasureTargetOutcome;
use temper_substrate::writes::resolve_emitter;
use temper_workflow::operations::Surface;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};
use crate::services::erasure_slack_revocation_service::{self, SlackRevokeConfig};
use crate::services::resource_erasure_service::{
    is_retryable_act_error, operator_holds, operator_refusal, operator_still_holds,
    queue_region_settling, MAX_ACT_RETRIES,
};

/// One blob strike's verdict, exactly what the `blob_delete` wrapper returned. The provider
/// bytes themselves are not the service's business: a released verdict is drained by the
/// fence (`erasure_fence_service`), which derives its work from the `principal_erased`
/// payload (derive-don't-remember).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStrikeOutcome {
    pub blob_id: Uuid,
    pub released: bool,
}

/// A completed erasure — the full act or, on a re-erase, the no-op completion (spec §5:
/// `already_erased`). The no-op's targets are whatever live rows the estate still holds:
/// post-act re-commits into a retired home report strikes, not `already-erased`, so an
/// empty target set means an empty estate, not merely a tombstoned subject. Either way ONE
/// `principal_erased` event stands behind these values. This is the act's only answer: no door
/// raises a principal refusal (a non-admin is rejected at the wire, and a re-erase is this
/// no-op completion).
#[derive(Debug, Clone, PartialEq)]
pub struct ErasureCompletion {
    pub event_id: Uuid,
    /// The redacted set (D2): content hashes only — the governed-scope text hashes plus every
    /// hash actually struck. Team- and map-homed hashes are NOT here; they ride the targets
    /// as the named remainder.
    pub redacted_hashes: Vec<String>,
    /// Per-target outcomes and the named remainder (D6's accepted-in-part arm).
    pub targets: Vec<ErasureTargetOutcome>,
    pub blob_strikes: Vec<BlobStrikeOutcome>,
    /// True when the subject was already tombstoned: this completion erased nothing new.
    pub already_erased: bool,
    /// The resource erasures the act ran over the estate, in the order it ran them (D4).
    pub resource_erasures: Vec<EstateResourceErasure>,
    /// Live resources and live blobs still homed in the estate once the act committed (D9, ruled
    /// Q3). `None` when the count could not be read: unknown is never reported as zero.
    pub estate_stragglers: Option<u32>,
    /// What happened to the subject's Slack grants at Slack before the act destroyed them.
    pub slack_revocation: SlackRevocationTally,
}

/// The jsonb `principal_erasure_execute` returns, before mapping onto the typed outcome.
#[derive(Debug, serde::Deserialize)]
struct ExecuteOutcomeWire {
    event_id: Uuid,
    redacted_hashes: Vec<String>,
    targets: Vec<ErasureTargetOutcome>,
    already_erased: bool,
    estate_contexts: Vec<Uuid>,
    resource_erasures: Vec<ResourceErasureWire>,
}

/// One entry of the act's `resource_erasures`, as the ledger spells it: `resource` and `event`,
/// never `resource_id`, because the completion payload carries no trail join key.
#[derive(Debug, serde::Deserialize)]
struct ResourceErasureWire {
    resource: Uuid,
    event: Uuid,
    kind: EstateErasureKind,
}

/// One blob strike's PREDICTED verdict from the survey — the plan's `released_would_be`:
/// the `blob_delete` refcount's own predicate, run at survey time. THE STRIKE-TIME VERDICT
/// IS AUTHORITATIVE (the `strike_verdicts` honesty rule): this speaks for the moment the
/// survey ran, and a commit or a sibling strike in between can flip the act's actual
/// verdict; neither overrules the other.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BlobStrikeVerdict {
    pub blob_id: Uuid,
    pub released: bool,
}

/// What the read-only survey predicts the act would do (task 01a09628 item 2): the full
/// record the act would write — targets as prose, exactly, in the act's own order — the
/// redacted set, the tombstone state it would report, and the typed strike predictions.
/// Writes nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct ErasureSurvey {
    pub subject: ProfileId,
    pub already_erased: bool,
    /// The redacted set (D2) the act would admit: the governed-scope text hashes plus every
    /// hash it would actually strike.
    pub redacted_hashes: Vec<String>,
    pub targets: Vec<ErasureTargetOutcome>,
    pub blob_strikes: Vec<BlobStrikeVerdict>,
    /// The estate's size by disposition (D6): the survey's first answer.
    pub estate: EstateCounts,
    /// Every estate resource, in the order the act would take it, with counts from its own survey.
    pub resources: Vec<EstateResourcePlan>,
}

/// The jsonb `principal_erasure_survey` returns, before mapping onto the typed survey.
#[derive(Debug, serde::Deserialize)]
struct SurveyOutcomeWire {
    redacted_hashes: Vec<String>,
    targets: Vec<ErasureTargetOutcome>,
    already_erased: bool,
    blob_strikes: Vec<BlobStrikeVerdict>,
    estate: EstateCounts,
    resources: Vec<EstateResourcePlan>,
}

/// What the act needs from the instance beyond the database: how to revoke Slack grants, and which
/// login provider this instance's tokens resolve through (`AUTH_PROVIDER_NAME`). `Default` is
/// neither: Slack grants are destroyed unrevoked and refresh tokens never bound to a profile are
/// not attributed, and the record says both.
#[derive(Debug, Default, Clone, Copy)]
pub struct ErasureEnv<'a> {
    pub slack: Option<SlackRevokeConfig<'a>>,
    pub auth_provider: Option<&'a str>,
}

/// Tell the act, for this transaction only, which login provider the subject's unbound refresh
/// tokens resolve through (`_erasure_auth_provider`).
async fn set_auth_provider(
    conn: &mut sqlx::PgConnection,
    provider: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT set_config('temper.erasure_auth_provider', $1, true)",
        provider.unwrap_or("")
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(())
}

/// Execute the erasure act for `subject`, as the operator `admin` names.
///
/// The [`SystemAdmin`] proof is the authority: the gate ran where the proof was minted, before
/// anything here, so the existence of the subject is disclosed only to an operator.
///
/// The request reference tolerates retries: a retried call with the SAME reference re-executes
/// as a no-op completion on an already-erased subject (`already_erased`, spec §5). Correlation
/// is INDEXED, never unique (20260624000001_canonical_schema.sql:491) — the reference pairs
/// the act's events, it does not deduplicate the door.
pub async fn execute_erasure(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
    request_reference: Uuid,
    surface: Surface,
    env: &ErasureEnv<'_>,
) -> ApiResult<ErasureCompletion> {
    let operator = admin.actor();
    // The operator's standing first, under the governance lock, before anything is looked up or
    // revoked at Slack: a demoted admin holding a stale proof is refused (`404`, what a non-admin
    // is told), one whose demotion is in flight waits and is then refused, and in either case
    // nothing reaches Slack. The gate commits at once, so no transaction is held across the Slack
    // calls. Each act attempt asks again under the lock, and that is the check that holds to the
    // act's commit; a demotion committing between the gate and the act can leave the grants
    // revoked at Slack with the act refused, the state a failed act already leaves (see the
    // failure arm), and nothing else.
    let mut gate = pool.begin().await?;
    operator_holds(&mut gate, operator).await?;
    gate.commit().await?;

    // The emitter resolves on the pool, before the transaction opens (a read, not part of the
    // mutation) — the operator's entity on the door's surface. Hard precondition, the
    // `slack_disconnect_service` shape: an unattributable authority act is worse than a failed
    // one. The surface rides from the door (`Surface::ApiHttp` from the HTTP door, the one that
    // exists today) so the ledger says where the act came from, never a hard-wired guess that
    // outlives its accuracy — the blob commit's S5 correction.
    let emitter = resolve_emitter(pool, operator, surface.marker())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Operator-only from here: existence may be disclosed (and as an error, not a ledger row).
    let exists: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT id FROM kb_profiles WHERE id = $1"#,
        subject.uuid()
    )
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("profile not found".to_string()));
    }

    // The subject's Slack grants are revoked at Slack BEFORE the act, while their ciphertext still
    // exists: the act destroys it, and Slack's revoke takes the token itself (ruled 2026-10-10).
    // Once, outside the retry loop below, so a retried act never re-asks Slack. A grant whose
    // revoke failed is queued for the drain in each attempt's own transaction, beside the act, so
    // a failed attempt queues nothing. `slack` is `None` where linking is not configured: no grant
    // can be opened, and the tally says how many were destroyed unrevoked. A grant linked between
    // this read and the act is destroyed unrevoked and uncounted: a narrow race, named here so the
    // tally is not read as complete.
    let (slack_revocation, pending) = erasure_slack_revocation_service::revoke_before_erasure(
        pool,
        subject.uuid(),
        env.slack.as_ref(),
    )
    .await?;

    // The act runs resource erasure once per estate resource (20261021100000), so it fails
    // retryably where that act does: a deadlock, a raced edge fold, a remote source that gained or
    // lost a citer, a collision on the redaction rows (D8). The act is one statement and one
    // transaction, so a failed attempt commits nothing and each retry is a fresh statement against
    // the post-conflict state.
    let mut retries = 0;
    let raw = loop {
        // Each attempt is its own transaction, the operator re-checked first under its lock.
        let mut tx = pool.begin().await?;
        let result = async {
            // `None`: the operator is no longer a system admin; the act is not run.
            if !operator_still_holds(&mut tx, operator).await? {
                return Ok(None);
            }
            set_auth_provider(&mut tx, env.auth_provider).await?;
            erasure_slack_revocation_service::queue_pending(&mut tx, &pending).await?;
            let raw = sqlx::query_scalar!(
                r#"SELECT principal_erasure_execute($1, $2, $3, $4) AS "outcome: serde_json::Value""#,
                subject.uuid(),
                operator.uuid(),
                emitter.uuid(),
                request_reference,
            )
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(Some(raw))
        }
        .await;
        match result {
            Ok(Some(raw)) => break raw,
            Ok(None) => {
                if slack_revocation.revoked > 0 {
                    tracing::warn!(
                        revoked = slack_revocation.revoked,
                        "principal erasure refused after revoking the subject's Slack grants at \
                         Slack: the operator is no longer a system admin; re-run it as an admin \
                         to destroy temper's copies"
                    );
                }
                return Err(operator_refusal(operator));
            }
            Err(err) if is_retryable_act_error(&err) && retries < MAX_ACT_RETRIES => retries += 1,
            // Scrubbed, as the resource door does: an exhausted retry on a redaction-row
            // collision is a unique violation, which the generic mapping would render as a 409
            // "already exists" that misdescribes it.
            Err(err) => {
                // The grants revoked above stay revoked: Slack cannot un-revoke them. The vault rows
                // and links stand until the act is re-run, which destroys them.
                if slack_revocation.revoked > 0 {
                    tracing::warn!(
                        revoked = slack_revocation.revoked,
                        "principal erasure failed after revoking the subject's Slack grants at \
                         Slack; re-run the erasure to destroy temper's copies"
                    );
                }
                return Err(ApiError::internal_scrubbed(
                    "principal erasure act failed",
                    err,
                ));
            }
        }
    }
    .ok_or_else(|| ApiError::Internal("principal_erasure_execute returned no row".to_string()))?;
    let wire: ExecuteOutcomeWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("erasure outcome shape: {e}")))?;

    // COMMITTED. Nothing below may answer as a failure: the operator would re-run an act that
    // succeeded. Region settling for every erased resource's anchors, as the resource door does
    // after its own act (each erasure nulled those watermarks and recomputed the live centroids;
    // the settling re-forms the regions without the husk). Never fails the act.
    for e in &wire.resource_erasures {
        queue_region_settling(pool, ResourceId::from(e.resource)).await;
    }
    let estate_stragglers = estate_stragglers(pool, subject.uuid(), &wire.estate_contexts).await;

    // The per-row verdicts live in the payload's targets; the strikes are re-matched out of
    // them so the typed outcome can carry what a fence needs without re-deriving from prose.
    // The SQL folded `blob_delete`'s verdict into each strike's outcome string; the
    // authoritative per-row record is the `blob_erased` events, which share the completion's
    // correlation id.
    let blob_strikes = strike_verdicts(pool, wire.event_id).await?;

    Ok(ErasureCompletion {
        event_id: wire.event_id,
        redacted_hashes: wire.redacted_hashes,
        targets: wire.targets,
        blob_strikes,
        already_erased: wire.already_erased,
        slack_revocation,
        resource_erasures: wire
            .resource_erasures
            .into_iter()
            .map(|e| EstateResourceErasure {
                resource_id: e.resource,
                event_id: e.event,
                kind: e.kind,
            })
            .collect(),
        estate_stragglers,
    })
}

/// Live resources and live blobs still homed in the estate contexts the act recorded, charters
/// aside: one created there after the act's plan read the estate and committed before the act
/// retired the context (D9). Plus every context of the subject's estate, as it stands now, with
/// no `context_erased`: one created in their `@me` or personal team while the act ran (R7).
/// Ruled Q3 (2026-10-10): reported, not locked against. Re-running the act erases them. A failed read is logged and answered `None`, never zero: the act has
/// committed, so it must not fail, and an unknown count must not read as a clean estate.
async fn estate_stragglers(pool: &PgPool, subject: Uuid, estate: &[Uuid]) -> Option<u32> {
    let count = sqlx::query_scalar!(
        r#"SELECT (SELECT count(*)
                     FROM _erasure_estate_contexts($2) g
                    WHERE NOT EXISTS (SELECT 1 FROM kb_events e
                                        JOIN kb_event_types t ON t.id = e.event_type_id
                                       WHERE t.name = 'context_erased'
                                         AND e.producing_anchor_table = 'kb_contexts'
                                         AND e.producing_anchor_id = g))
                + (SELECT count(*)
                     FROM kb_resource_homes h
                     JOIN kb_resources r ON r.id = h.resource_id
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY($1)
                      AND r.erased_at IS NULL
                      AND NOT EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.telos_resource_id = r.id))
                + (SELECT count(*)
                     FROM kb_blobs b
                    WHERE b.home_table = 'kb_contexts'
                      AND b.home_id = ANY($1)
                      AND b.content_type IS NOT NULL) AS "n!""#,
        estate,
        subject,
    )
    .fetch_one(pool)
    .await;
    match count {
        Ok(n) => Some(u32::try_from(n).unwrap_or(u32::MAX)),
        Err(e) => {
            tracing::warn!(error = %e, "failed to count the estate's remaining live resources and blobs after erasure; reported as unknown");
            None
        }
    }
}

/// The per-row strike verdicts for a completion: every `blob_erased` event sharing the
/// completion's correlation id, joined to the row it emptied, each carrying the verdict the
/// wrapper decided AT THAT STRIKE'S MOMENT. A later read sees only the end state, so the
/// moment is reconstructed: live rows when the wrapper counted = live rows now + same-hash
/// strikes LATER in the same act (each emptied one live row) + the struck row itself, which
/// was still live under its own refcount — so released ⟺ live-now + later = 0. A subject's
/// two same-hash homes therefore read false then true, the act's sequential refcount, not a
/// flat end-state read. THE ORDER IS THE ACT'S OWN: the strike loop consumes the plan's
/// rows in `kb_blobs.id` order (the plan's declared, load-bearing order), so "later" is a
/// key comparison on the struck row's id — exact regardless of event-timestamp or
/// uuid-generation ordering, which tie at transaction-stable timestamps. The fence derives
/// its own pathname from the payload's per-target outcome prose — the STRUCK row's
/// `blob_pathname` is always NULL (the strike emptied it), so it is not read here.
async fn strike_verdicts(
    pool: &PgPool,
    completion_event: Uuid,
) -> ApiResult<Vec<BlobStrikeOutcome>> {
    let rows = sqlx::query!(
        r#"
        SELECT (e.payload->>'blob_id')::uuid       AS "blob_id: Uuid",
               ((SELECT count(*) FROM kb_blobs live
                  WHERE live.content_hash = b.content_hash
                    AND live.content_type IS NOT NULL)
                + (SELECT count(*) FROM kb_events f
                    JOIN kb_event_types ft ON ft.id = f.event_type_id
                          AND ft.name = 'blob_erased'
                   WHERE f.correlation_id = e.correlation_id
                     AND f.id <> e.id
                     AND (f.payload->>'blob_id')::uuid IN (
                         SELECT s.id FROM kb_blobs s
                          WHERE s.content_hash = b.content_hash
                            AND s.id > b.id))) = 0 AS "released: bool"
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id AND t.name = 'blob_erased'
          JOIN kb_blobs b ON b.id = (e.payload->>'blob_id')::uuid
         WHERE e.correlation_id = (SELECT correlation_id FROM kb_events WHERE id = $1)
           AND e.id <> $1
         ORDER BY b.id
        "#,
        completion_event,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BlobStrikeOutcome {
            blob_id: r.blob_id.expect("a blob_erased payload carries blob_id"),
            // The wrapper's verdict at ITS moment — the struck row was live under its own
            // refcount, so live-now + later same-hash strikes = 0 ⟺ it was the last live
            // row. A re-commit after the act (the fence's declared-open window) drifts this
            // read from the strike-time prose, and the payload's prose stays authoritative.
            released: r.released.unwrap_or(false),
        })
        .collect())
}

/// The read-only survey (task 01a09628 item 2): what [`execute_erasure`] WOULD record if it
/// ran now. The prediction is the act's OWN computation — `principal_erasure_survey_plan`
/// (migration 20260913000010), the exact body the act consumes — never a re-derivation, and
/// its strike verdicts simulate the act's own sequential refcount (same-hash rows earlier
/// in the strike set are already emptied when the act reaches this row). In the same state,
/// the prediction matches the record; the strike-time verdict stays authoritative for a
/// commit or sibling strike that lands after the survey (a preview that can disagree is
/// worse than no preview).
///
/// The [`SystemAdmin`] proof is the gate, as for the act. The survey writes nothing, so there is
/// deliberately no `Surface` parameter: it appends nothing, so there is nothing to attribute
/// (ruled 2026-09-12: a survey attempt is not an erasure request).
///
/// Existence is disclosed only to an operator (the execute door's order).
pub async fn survey_erasure(
    pool: &PgPool,
    _admin: &SystemAdmin,
    subject: ProfileId,
    auth_provider: Option<&str>,
) -> ApiResult<ErasureSurvey> {
    // Operator-only from here: existence may be disclosed (and as an error, not a ledger row).
    let exists: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT id FROM kb_profiles WHERE id = $1"#,
        subject.uuid()
    )
    .fetch_optional(pool)
    .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound("profile not found".to_string()));
    }

    let mut tx = pool.begin().await?;
    set_auth_provider(&mut tx, auth_provider).await?;
    let raw = sqlx::query_scalar!(
        r#"SELECT principal_erasure_survey($1) AS "survey: serde_json::Value""#,
        subject.uuid(),
    )
    .fetch_one(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::Internal("principal_erasure_survey returned no row".to_string()))?;
    let wire: SurveyOutcomeWire = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("erasure survey shape: {e}")))?;

    Ok(ErasureSurvey {
        subject,
        already_erased: wire.already_erased,
        redacted_hashes: wire.redacted_hashes,
        targets: wire.targets,
        blob_strikes: wire.blob_strikes,
        estate: wire.estate,
        resources: wire.resources,
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
