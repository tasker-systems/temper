//! Delivery lifecycle — the projection of a matched radius into per-subscriber rows, and the two
//! legs (scope, then judgment) walked over them. S2 chunk C of "external systems as subscribed
//! emitters".
//!
//! **The delivery row is a projection, not a second event.** One webhook receipt is one ledger
//! event, always (goal C2); the delivery rows are *outcomes of processing* that event. They are
//! written in Rust inside the intake transaction, which CONFORMS to the precedent the goal itself
//! cites: `region_materialized` writes its N member rows in the event's own transaction
//! (`temper-substrate/src/write.rs:196-204`) while its `_project_*` half only stamps a watermark.
//! A payload-first projection half is unavailable here — the halves read only the payload
//! (`canonical_schema.sql:473`) and chunk B's payload is the remote's verbatim body, which does
//! not carry the matched set.
//!
//! **Two lifecycles share this table.** `kb_invocations.originating_cogmap_id` is `NOT NULL`, so
//! only a `kb_cogmaps` subscriber can ever be acted for under an invocation envelope. A
//! `kb_contexts` or `kb_teams` subscriber has no telos, no steward and no envelope: it subscribed
//! in order to **be aware**, and its delivery is terminal at `in_scope`/`undetermined`. Nothing
//! here requires a disposition; the constraints govern what one must CARRY, never that one must be
//! made. Reporting an awareness-only subscriber's undisposed rows as backlog is forbidden by the
//! goal's `No phantom backlog` negative — which is why there is no `outstanding_count` verb on
//! this module, and why [`list_for_subscription`] returns rows rather than a queue depth.
//!
//! **This module is also the read surface** (goal C11). A `webhook_received` event is unreachable
//! on all three existing `kb_events` read paths, so without [`list_for_subscription`] chunk B's
//! routing stays written and unreadable by the party it named.
//!
//! Authorization is not re-invented: every verb gates through
//! [`subscription_service::get_for_caller`], so a delivery's readability is exactly its
//! subscription's. That is the correct derivation — a delivery is a fact *about* a declaration,
//! and there is no sense in which it could be more or less visible than the declaration itself.

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use temper_core::types::delivery::{
    DeclarationLiveness, Delivery, DeliveryStatus, Disposition, RecordDispositionRequest,
    RecordScopeRequest,
};
use temper_core::types::ids::ProfileId;
use temper_substrate::payloads::{AnchorTable, EventRef, RefRel, RefTarget};

use crate::error::{ApiError, ApiResult};
use crate::services::{connection_service, subscription_service};

/// The event type a disposition is authored as. Registered TYPED (published `payload_schema` from
/// the committed schemars snapshot) by migration `20260819000030` — unlike `webhook_received`,
/// this is temper's own act with a shape temper controls, so the permissive path does not apply.
const DISPOSED_TYPE: &str = "subscription_delivery_disposed";

/// The raw row, before the two string columns become typed enums. `query_as!` cannot map a `TEXT`
/// column onto a Rust enum, and a runtime `query_as` would lose compile-time checking — so the
/// macro stays and the narrowing happens in [`DeliveryRow::into_delivery`], where an unparseable
/// value is a loud `Internal` rather than a silent default.
struct DeliveryRow {
    id: Uuid,
    subscription_id: Uuid,
    event_id: Uuid,
    status: String,
    scope_reason: Option<String>,
    scoped_at: Option<chrono::DateTime<chrono::Utc>>,
    disposition: Option<String>,
    decided_by_event_id: Option<Uuid>,
    decided_by_invocation_id: Option<Uuid>,
    decided_by_profile_id: Option<Uuid>,
    decided_at: Option<chrono::DateTime<chrono::Utc>>,
    rationale: Option<String>,
    confidence: Option<f64>,
    created: chrono::DateTime<chrono::Utc>,
}

impl DeliveryRow {
    fn into_delivery(self) -> ApiResult<Delivery> {
        let status = match self.status.as_str() {
            "pending_scope" => DeliveryStatus::PendingScope,
            "in_scope" => DeliveryStatus::InScope,
            "out_of_scope" => DeliveryStatus::OutOfScope,
            "undetermined" => DeliveryStatus::Undetermined,
            // The CHECK on the column makes this unreachable for rows in the table. It is an
            // Internal rather than a default because a status we cannot name is precisely the
            // case where guessing would collapse `undetermined` into something quieter.
            other => {
                return Err(ApiError::Internal(format!(
                    "delivery {} has unknown status '{other}'",
                    self.id
                )))
            }
        };
        let disposition = match self.disposition.as_deref() {
            None => None,
            Some("acted") => Some(Disposition::Acted),
            Some("declined") => Some(Disposition::Declined),
            Some(other) => {
                return Err(ApiError::Internal(format!(
                    "delivery {} has unknown disposition '{other}'",
                    self.id
                )))
            }
        };
        Ok(Delivery {
            id: self.id,
            subscription_id: self.subscription_id,
            event_id: self.event_id,
            status,
            scope_reason: self.scope_reason,
            scoped_at: self.scoped_at,
            disposition,
            decided_by_event_id: self.decided_by_event_id,
            decided_by_invocation_id: self.decided_by_invocation_id,
            decided_by_profile_id: self.decided_by_profile_id,
            decided_at: self.decided_at,
            rationale: self.rationale,
            confidence: self.confidence,
            created: self.created,
        })
    }
}

/// Project one delivery row per matched subscription, **inside the caller's transaction**.
///
/// Called by `intake_service::receive_webhook` in the same transaction as the `_event_append`, so
/// a webhook either lands as one event with its full fan of delivery rows or lands not at all.
/// There is no window in which the routing exists and the deliveries do not.
///
/// A payload matching zero subscriptions projects **zero** rows and this is a no-op returning 0 —
/// the empty radius is the noise filter (goal C4), and a routed-nowhere payload is a well-formed
/// act the system said no to, not an error.
///
/// `ON CONFLICT DO NOTHING` against the `(subscription_id, event_id)` unique constraint makes the
/// projection idempotent under retry. It is not papering over a double-projection bug: intake
/// computes the matched set once per event, so a conflict can only arise from a replayed call.
pub(crate) async fn project(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    subscription_ids: &[Uuid],
) -> ApiResult<u64> {
    if subscription_ids.is_empty() {
        return Ok(0);
    }
    // One statement via UNNEST rather than a loop: N round-trips for a fan-out that is already
    // known in full is waste, and the set is bounded by the connection's live subscriptions.
    let result = sqlx::query!(
        r#"INSERT INTO kb_subscription_deliveries (subscription_id, event_id)
           SELECT s, $2 FROM unnest($1::uuid[]) AS s
           ON CONFLICT (subscription_id, event_id) DO NOTHING"#,
        subscription_ids,
        event_id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Load one delivery by id, unauthorized. The internal primitive readbacks use; surface callers
/// want [`get_for_caller`].
pub async fn get(pool: &PgPool, id: Uuid) -> ApiResult<Delivery> {
    sqlx::query_as!(
        DeliveryRow,
        r#"SELECT id, subscription_id, event_id, status,
                  scope_reason, scoped_at,
                  disposition, decided_by_event_id, decided_by_invocation_id,
                  decided_by_profile_id, decided_at, rationale, confidence, created
             FROM kb_subscription_deliveries WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("delivery not found or not readable".to_string()))?
    .into_delivery()
}

/// [`get`], gated through the delivery's own subscription.
pub async fn get_for_caller(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<Delivery> {
    let delivery = get(pool, id).await?;
    subscription_service::get_for_caller(pool, caller, delivery.subscription_id).await?;
    Ok(delivery)
}

/// **Goal C11** — what was routed to this declaration, and what became of it.
///
/// Newest first by uuidv7 id, whose byte order IS time order (the same property
/// `steward_ingest_delta` relies on for `max_event_id`), served by
/// `idx_kb_subscription_deliveries_subscription`.
///
/// Returns rows, deliberately, and not a count of undisposed ones. For an awareness-only
/// subscriber an undisposed delivery is a terminal state; a `pending_count` verb here would be the
/// `No phantom backlog` negative implemented as an API.
pub async fn list_for_subscription(
    pool: &PgPool,
    caller: ProfileId,
    subscription_id: Uuid,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<Delivery>> {
    subscription_service::get_for_caller(pool, caller, subscription_id).await?;
    let rows = sqlx::query_as!(
        DeliveryRow,
        r#"SELECT id, subscription_id, event_id, status,
                  scope_reason, scoped_at,
                  disposition, decided_by_event_id, decided_by_invocation_id,
                  decided_by_profile_id, decided_at, rationale, confidence, created
             FROM kb_subscription_deliveries
            WHERE subscription_id = $1
            ORDER BY id DESC
            LIMIT $2 OFFSET $3"#,
        subscription_id,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(DeliveryRow::into_delivery).collect()
}

/// The DLQ read: deliveries stuck at `undetermined`, which invariant 6 requires stay **visible**.
///
/// Separate from [`list_for_subscription`] because "is anything stuck?" is a question two of the
/// three subscriber kinds have no agent to ask on their behalf. Served by the partial index
/// `idx_kb_subscription_deliveries_undetermined`, whose predicate matches this `WHERE` exactly.
///
/// **A judged delivery has left the DLQ.** `undetermined` admits judgment
/// ([`DeliveryStatus::is_surfaced_for_judgment`]), so a steward can decline an enrichment failure
/// on its merits — and once it has, the delivery is resolved and must stop being reported as
/// stuck. Invariant 6 wants the DLQ *visible*; it does not want it monotonically increasing, and a
/// queue that can only grow is one nobody reads. The status stays `undetermined` deliberately: it
/// remains the true statement about what enrichment concluded, and rewriting it on disposition
/// would destroy the record of why the judgment was made under uncertainty.
pub async fn list_undetermined(
    pool: &PgPool,
    caller: ProfileId,
    subscription_id: Uuid,
) -> ApiResult<Vec<Delivery>> {
    subscription_service::get_for_caller(pool, caller, subscription_id).await?;
    let rows = sqlx::query_as!(
        DeliveryRow,
        r#"SELECT id, subscription_id, event_id, status,
                  scope_reason, scoped_at,
                  disposition, decided_by_event_id, decided_by_invocation_id,
                  decided_by_profile_id, decided_at, rationale, confidence, created
             FROM kb_subscription_deliveries
            WHERE subscription_id = $1
              AND status = 'undetermined'
              AND disposition IS NULL
            ORDER BY id DESC"#,
        subscription_id,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(DeliveryRow::into_delivery).collect()
}

/// **Goal C12** — tell a declaration that has never matched from a source that has been quiet.
///
/// The clause is a claim about **absence**, so the delivery table cannot answer it alone: a
/// declaration that never matched has no row to count. This crosses three facts that already exist
/// and adds **no write to the intake path** — a rejected alternative was stamping a
/// `last_evaluated_at` on every subscription per intake, which would have put a write on the read
/// path and made `kb_subscriptions` mutable per event to record something already derivable.
///
/// The ordering of the arms is the discriminator, and it is deliberate: revocation first (it
/// explains everything downstream of it), then the delivery count (a matching declaration needs no
/// further explanation), then the connection's credential, and only then the source's own
/// quietness. Reading them in any other order would report a broken connection as a quiet one.
pub async fn declaration_liveness(
    pool: &PgPool,
    caller: ProfileId,
    subscription_id: Uuid,
) -> ApiResult<DeclarationLiveness> {
    let sub = subscription_service::get_for_caller(pool, caller, subscription_id).await?;

    if let Some(revoked_at) = sub.revoked_at {
        return Ok(DeclarationLiveness::Revoked { revoked_at });
    }

    let delivery_count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM kb_subscription_deliveries WHERE subscription_id = $1"#,
        subscription_id,
    )
    .fetch_one(pool)
    .await?;
    if delivery_count > 0 {
        return Ok(DeclarationLiveness::Matching { delivery_count });
    }

    let conn = connection_service::get(pool, sub.connection_id).await?;

    // How much has actually reached this connection SINCE THIS DECLARATION EXISTED? Scoped by the
    // connection's emitter entity and served by
    // `idx_kb_events_emitter ON kb_events(emitter_entity_id, occurred_at DESC)`
    // (canonical_schema.sql:489) — the index already exists for exactly this shape, both legs.
    //
    // The time bound is load-bearing, not hygiene. The delivery count this is compared against can
    // only include events from after the subscription was created, so an all-time count compares
    // two different windows: a team adding a subscription today to a connection with a year of
    // history would be told `SelectorMatchesNothing` on its very first read — "the overwhelmingly
    // likely cause is the selector" — when nothing has arrived since the declaration existed and
    // the selector may be perfectly correct. The clause exists to stop a maintainer misreading
    // silence; an unwindowed count manufactures the opposite misreading.
    let events_on_connection = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!"
             FROM kb_events
            WHERE emitter_entity_id = $1
              AND occurred_at >= $2"#,
        conn.emitter_entity_id,
        sub.created,
    )
    .fetch_one(pool)
    .await?;

    if events_on_connection > 0 {
        // Payloads arrived and this selector matched none of them. Said plainly, because the
        // overwhelmingly likely cause is the selector and the whole point of the clause is to stop
        // a maintainer reading that as "the repo has been quiet".
        return Ok(DeclarationLiveness::SelectorMatchesNothing {
            events_on_connection,
        });
    }

    // Nothing arrived. `credential IS NULL` is the `needs_credential` birth state, per the
    // column's own comment — so this separates "the connection was never going to receive
    // anything" from "the source genuinely had nothing to say".
    if conn.credential.is_none() {
        return Ok(DeclarationLiveness::ConnectionNotAuthenticated);
    }
    Ok(DeclarationLiveness::SourceQuiet)
}

/// Record the outcome of scoping a delivery — the enrichment leg (S4 is the caller).
///
/// Ships in chunk C so the state machine has a witness rather than a hole between intake and a
/// chunk that does not exist yet.
///
/// Two refusals live here rather than in the schema, because a CHECK violation surfaced as a 500
/// tells the caller nothing:
/// - `undetermined` without a reason. Invariant 6 wants the DLQ **legible**, not merely visible.
/// - re-scoping a delivery that has already been judged. The judgment was made against a scope;
///   moving the scope underneath it would silently invalidate the reasoning on the row.
///
/// Re-scoping an `undetermined` delivery IS allowed — a retried enrichment resolving the DLQ is
/// exactly the recovery path invariant 6 anticipates. What must never happen is that transition
/// occurring *silently*, and it cannot: `scope_reason` is overwritten with the new reason and
/// `scoped_at` moves, both on the ledger-visible row.
pub async fn record_scope(
    pool: &PgPool,
    caller: ProfileId,
    delivery_id: Uuid,
    req: &RecordScopeRequest,
) -> ApiResult<Delivery> {
    if req.status == DeliveryStatus::PendingScope {
        return Err(ApiError::BadRequest(
            "pending_scope is the birth state; it cannot be recorded as a scope outcome".into(),
        ));
    }
    if req.status == DeliveryStatus::Undetermined && req.reason.as_deref().unwrap_or("").is_empty()
    {
        return Err(ApiError::BadRequest(
            "an undetermined delivery must say why: invariant 6 requires the DLQ be legible, \
             not merely visible"
                .into(),
        ));
    }

    // Auth before writes, derived from the delivery's own subscription.
    let existing = get(pool, delivery_id).await?;
    subscription_service::get_for_caller(pool, caller, existing.subscription_id).await?;

    if existing.disposition.is_some() {
        return Err(ApiError::BadRequest(
            "this delivery has already been judged; re-scoping it would invalidate the \
             reasoning recorded against the scope that was judged"
                .into(),
        ));
    }

    // `AND disposition IS NULL` is the guard, not the check above. The check reads a snapshot on
    // the pool; between it and this write a steward can judge the delivery, and the interleaving
    // is not hypothetical in the one direction the schema does NOT rescue: writing `undetermined`
    // with a reason satisfies every CHECK, so without this predicate the row would end up
    // carrying a judgment made against `in_scope` while reading `undetermined` — the scope moved
    // underneath the reasoning, exactly as this function's docs say must never happen. (Writing
    // `out_of_scope` would be caught by `disposition_follows_a_resolved_scope`, but as a 500
    // rather than the answer the caller deserves.)
    let updated = sqlx::query!(
        r#"UPDATE kb_subscription_deliveries
              SET status = $2, scope_reason = $3, scoped_at = now()
            WHERE id = $1 AND disposition IS NULL"#,
        delivery_id,
        req.status.as_str(),
        req.reason.as_deref(),
    )
    .execute(pool)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::BadRequest(
            "this delivery was judged concurrently; re-scoping it would invalidate the              reasoning recorded against the scope that was judged"
                .into(),
        ));
    }

    get(pool, delivery_id).await
}

/// Author a judgment on a delivery — the disposition act.
///
/// **This is an act, not a column write.** A `subscription_delivery_disposed` event is appended to
/// the ledger carrying the reasoning and confidence, and the delivery row's `rationale` /
/// `confidence` are a queryable projection of it. A steward declining a PR is accountable and
/// citable: *"the platform team's steward saw PR #412, judged it immaterial at confidence 0.7,
/// because it touched only test fixtures."*
///
/// **The accountability carrier is the actor, not the envelope.** `invocation_id` is optional: an
/// agent acting for a cogmap passes one, a human does not, and the acting profile carries
/// attribution in both cases. Requiring an envelope would make this verb unreachable for
/// `kb_contexts` and `kb_teams` subscribers, because `kb_invocations.originating_cogmap_id` is
/// `NOT NULL`.
///
/// The event and the row move in one transaction. A judgment visible on the ledger but not on the
/// row it judged — or the reverse — would be exactly the ambiguity the delivery table exists to
/// remove.
pub async fn record_disposition(
    pool: &PgPool,
    caller: ProfileId,
    delivery_id: Uuid,
    req: &RecordDispositionRequest,
) -> ApiResult<Delivery> {
    if req.rationale.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "a disposition must carry its reasoning — a decline without it is the silent cursor \
             bump the delivery row exists to prevent"
                .into(),
        ));
    }
    if !(0.0..=1.0).contains(&req.confidence) {
        return Err(ApiError::BadRequest(format!(
            "confidence must be in [0,1], got {}",
            req.confidence
        )));
    }

    // Auth before writes.
    let existing = get(pool, delivery_id).await?;
    let sub = subscription_service::get_for_caller(pool, caller, existing.subscription_id).await?;

    // Judgment only follows a resolved scope, and only where there is something to judge. The
    // schema enforces this too (`disposition_follows_a_resolved_scope`); refusing here turns a
    // constraint violation into an answer the caller can act on.
    if !existing.status.is_surfaced_for_judgment() {
        return Err(ApiError::BadRequest(format!(
            "a delivery at '{}' cannot be judged: pending_scope has not been scoped yet, and \
             out_of_scope was determined not to touch this subscriber",
            existing.status.as_str()
        )));
    }
    if existing.disposition.is_some() {
        return Err(ApiError::BadRequest(
            "this delivery has already been judged; supersede it with a new act rather than \
             overwriting the record of the first"
                .into(),
        ));
    }

    // An envelope is only meaningful when an agent is acting FOR the cogmap that subscribed, and
    // the column's whole argument is accountability — an unvalidated citation is neither
    // accountable nor citable. The FK to `kb_invocations` proves the envelope exists; it does not
    // prove it is *this* subscriber's. Two things are checked, and both follow directly from why
    // the column is nullable in the first place:
    //   1. Only a cogmap subscriber can be acted for under an envelope
    //      (`kb_invocations.originating_cogmap_id` is NOT NULL), so an envelope offered for a
    //      context or team subscriber is incoherent rather than merely unverified.
    //   2. The envelope's originating cogmap must BE the subscriber. Without this a maintainer
    //      could attribute their judgment to an unrelated team's run, and both the ledger event
    //      and the row would carry it.
    if let Some(invocation_id) = req.invocation_id {
        if sub.subscriber_table != "kb_cogmaps" {
            return Err(ApiError::BadRequest(format!(
                "an invocation envelope is only meaningful for a kb_cogmaps subscriber; this                  delivery's subscriber is a {}, which has no telos to act under. Dispose as a                  profile instead — the acting profile carries attribution.",
                sub.subscriber_table
            )));
        }
        let originating: Option<Uuid> = sqlx::query_scalar!(
            "SELECT originating_cogmap_id FROM kb_invocations WHERE id = $1",
            invocation_id,
        )
        .fetch_optional(pool)
        .await?;
        match originating {
            None => {
                return Err(ApiError::BadRequest(
                    "no such invocation".into(),
                ))
            }
            Some(cogmap) if cogmap != sub.subscriber_id => {
                return Err(ApiError::BadRequest(
                    "that invocation originates from a different cogmap than this delivery's                      subscriber; a judgment may only cite the envelope it was actually made under"
                        .into(),
                ))
            }
            Some(_) => {}
        }
    }

    // Resolved on the pool, before the transaction opens: the emitter is a read, and a missing one
    // means the actor's own profile is structurally incomplete. Same posture as
    // `slack_disconnect_service` — no system fallback, because the ledger derives authorship FROM
    // the emitter and a fallback would put a fabricated attribution on it.
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // The judgment anchors where the receipt does — the connection's home context. One anchor per
    // event, and the subscriber rides `references` as a `touches`, which is what puts the judgment
    // in front of the same sweep that surfaced the delivery.
    let conn = connection_service::get(pool, sub.connection_id).await?;
    let subscriber_kind = subscriber_table_to_anchor(&sub.subscriber_table)?;
    let references = serde_json::to_value(vec![EventRef {
        rel: RefRel::Touches,
        target: RefTarget {
            kind: subscriber_kind,
            id: sub.subscriber_id,
        },
    }])
    .map_err(|e| ApiError::Internal(format!("references serialization failed: {e}")))?;

    let payload = serde_json::to_value(temper_substrate::payloads::SubscriptionDeliveryDisposed {
        delivery_id,
        subscription_id: sub.id,
        event_id: existing.event_id,
        disposition: req.disposition.as_str().to_string(),
        rationale: req.rationale.clone(),
        confidence: req.confidence,
        decided_by_profile_id: caller,
        decided_by_invocation_id: req.invocation_id,
    })
    .map_err(|e| ApiError::Internal(format!("payload serialization failed: {e}")))?;

    let mut tx = pool.begin().await?;

    // RE-READ UNDER A ROW LOCK. Every check above ran against a snapshot taken on the pool, and
    // the window between that read and this write is real: two callers disposing the same
    // delivery concurrently would both see `disposition = None`, both append a judgment event,
    // and leave the ledger carrying two judgment acts for one delivery while the row cites only
    // the second. That is precisely what the "supersede it with a new act rather than overwriting
    // the record of the first" refusal exists to prevent, so the guard has to hold under
    // concurrency or it does not hold at all.
    let locked = sqlx::query!(
        r#"SELECT status, disposition
             FROM kb_subscription_deliveries
            WHERE id = $1
              FOR UPDATE"#,
        delivery_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::NotFound("delivery not found or not readable".to_string()))?;

    if locked.disposition.is_some() {
        return Err(ApiError::BadRequest(
            "this delivery has already been judged; supersede it with a new act rather than              overwriting the record of the first"
                .into(),
        ));
    }
    if !matches!(locked.status.as_str(), "in_scope" | "undetermined") {
        return Err(ApiError::BadRequest(format!(
            "a delivery at '{}' cannot be judged: pending_scope has not been scoped yet, and              out_of_scope was determined not to touch this subscriber",
            locked.status
        )));
    }

    let event_id: Uuid = sqlx::query_scalar!(
        "SELECT _event_append($1, $2, 'kb_contexts', $3, $4, $5, $6, $7, $8, $9)",
        DISPOSED_TYPE,
        emitter.uuid(),
        conn.home_context_id,
        payload,
        references,
        None::<Uuid> as Option<Uuid>, // correlation: self-roots inside _event_append
        1i32,                         // payload_version
        serde_json::json!({}),        // metadata
        req.invocation_id,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(format!("_event_append failed: {e}")))?
    .expect("_event_append always returns a uuid");

    let updated = sqlx::query!(
        r#"UPDATE kb_subscription_deliveries
              SET disposition = $2,
                  rationale = $3,
                  confidence = $4,
                  decided_at = now(),
                  decided_by_event_id = $5,
                  decided_by_invocation_id = $6,
                  decided_by_profile_id = $7
            WHERE id = $1 AND disposition IS NULL"#,
        delivery_id,
        req.disposition.as_str(),
        req.rationale,
        req.confidence,
        event_id,
        req.invocation_id,
        *caller,
    )
    .execute(&mut *tx)
    .await?;

    // The row lock above makes this unreachable, and it is here anyway: the predicate and the
    // lock are two independent guards, and a silent zero-row UPDATE would commit the judgment
    // event with nothing pointing at it. Rolling back is what keeps the ledger and the row from
    // disagreeing.
    if updated.rows_affected() != 1 {
        return Err(ApiError::BadRequest(
            "this delivery was judged concurrently; the judgment was not recorded".into(),
        ));
    }

    tx.commit().await?;

    get(pool, delivery_id).await
}

/// Map a subscription's `subscriber_table` to the `AnchorTable` variant a `touches` rel carries.
/// The three admissible tables map 1:1; the CHECK on `kb_subscriptions.subscriber_table` makes an
/// unknown value unreachable for rows in the table, so this is `Internal` rather than a panic —
/// a stale string read out of the database should not take the process down.
fn subscriber_table_to_anchor(subscriber_table: &str) -> ApiResult<AnchorTable> {
    match subscriber_table {
        "kb_contexts" => Ok(AnchorTable::Contexts),
        "kb_cogmaps" => Ok(AnchorTable::Cogmaps),
        "kb_teams" => Ok(AnchorTable::Teams),
        other => Err(ApiError::Internal(format!(
            "subscriber_table '{other}' is not admissible; the CHECK on \
             kb_subscriptions.subscriber_table should have refused it"
        ))),
    }
}

// ── tests ───────────────────────────────────────────────────────────────────
//
// Run the tests you wrote or changed, the neighbouring ones in the same file/crate, and
// anything that regenerates a committed artifact. CI owns the broad suites.
// (`cargo nextest run -p temper-services --features test-db delivery_service`)

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use crate::services::intake_service;
    use crate::services::subscription_test_support::*;
    use sqlx::PgPool;
    use temper_core::types::subscription::SubscriptionSelector;

    /// The whole world one delivery needs: admin, team, team-owned context, connection with a
    /// reach grant, and a subscription. Returns (admin, team, context, connection, subscription).
    async fn seed_world(pool: &PgPool) -> (ProfileId, Uuid, Uuid, Uuid, Uuid) {
        let admin = seed_admin(pool).await;
        let team = seed_team(pool, admin).await;
        let ctx = seed_context_owned_by_team(pool, team).await;
        let conn = seed_connection(pool, Some(team), admin).await;
        grant_reach(pool, admin, conn, team).await;
        let sub = create_subscription(
            pool,
            admin,
            "kb_contexts",
            ctx,
            team,
            conn,
            SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec![],
            },
        )
        .await;
        (admin, team, ctx, conn, sub)
    }

    /// Drive one webhook through intake and return (event_id, the single delivery).
    async fn deliver_one(
        pool: &PgPool,
        conn: Uuid,
        sub: Uuid,
        admin: ProfileId,
    ) -> (Uuid, Delivery) {
        let event_id = intake_service::receive_webhook(
            pool,
            conn,
            "pull_request",
            &github_pr_payload(GITHUB_REPO),
        )
        .await
        .expect("receive webhook");
        let rows = list_for_subscription(pool, admin, sub, 50, 0)
            .await
            .expect("list deliveries");
        assert_eq!(rows.len(), 1, "expected exactly one delivery");
        (event_id, rows.into_iter().next().unwrap())
    }

    // ── the projection ──────────────────────────────────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn one_event_matching_n_subscriptions_projects_n_delivery_rows(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        // Three DISTINCT declarations against one connection. Distinct selectors, because the
        // unique index refuses the same selector twice for one authoring team.
        let mut subs = Vec::new();
        for repo in [GITHUB_REPO, "acme/other", GITHUB_REPO] {
            let ctx = seed_context_owned_by_team(&pool, team).await;
            let selector = if repo == GITHUB_REPO && !subs.is_empty() {
                SubscriptionSelector::GitHubCodeownersPaths {
                    repo: repo.into(),
                    paths: vec!["src/**".into()],
                }
            } else {
                SubscriptionSelector::GitHubRepository {
                    repo: repo.into(),
                    event_types: vec![],
                }
            };
            subs.push(
                create_subscription(&pool, admin, "kb_contexts", ctx, team, conn, selector).await,
            );
        }

        intake_service::receive_webhook(
            &pool,
            conn,
            "pull_request",
            &github_pr_payload(GITHUB_REPO),
        )
        .await
        .expect("receive webhook");

        // Two of the three match acme/temper; the acme/other declaration must not.
        let mut counts: Vec<usize> = Vec::new();
        for s in &subs {
            counts.push(
                list_for_subscription(&pool, admin, *s, 50, 0)
                    .await
                    .expect("list")
                    .len(),
            );
        }
        assert_eq!(
            counts,
            vec![1, 0, 1],
            "one delivery per MATCHED declaration, none for the rest"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_payload_matching_zero_subscriptions_projects_zero_delivery_rows(pool: PgPool) {
        let (admin, _team, _ctx, conn, sub) = seed_world(&pool).await;

        // A payload for a repo nothing declared an interest in. The empty radius is the noise
        // filter: the event is stored, and it routes nowhere.
        intake_service::receive_webhook(
            &pool,
            conn,
            "pull_request",
            &github_pr_payload("someone-else/unrelated"),
        )
        .await
        .expect("receive webhook");

        let rows = list_for_subscription(&pool, admin, sub, 50, 0)
            .await
            .expect("list");
        assert!(
            rows.is_empty(),
            "the empty radius must project no delivery rows"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_delivery_is_born_pending_scope(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (event_id, d) = deliver_one(&pool, conn, sub, admin).await;
        assert_eq!(d.status, DeliveryStatus::PendingScope);
        assert_eq!(
            d.event_id, event_id,
            "the delivery projects from the intake event"
        );
        assert!(d.disposition.is_none());
        assert!(d.scoped_at.is_none());
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_delivery_row_and_its_event_are_one_transaction(pool: PgPool) {
        // The delivery projects from the SAME event the radius was computed for — there is no
        // second event, and no window in which the routing exists without the rows that make it
        // readable. Witnessed by counting webhook_received events against delivery rows.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        deliver_one(&pool, conn, sub, admin).await;

        let events: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "c!" FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'webhook_received'"#
        )
        .fetch_one(&pool)
        .await
        .expect("count events");
        assert_eq!(events, 1, "one webhook receipt is one ledger event, always");
    }

    // ── the scope leg ───────────────────────────────────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn scope_walks_pending_to_in_scope(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        let scoped = record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("record scope");
        assert_eq!(scoped.status, DeliveryStatus::InScope);
        assert!(scoped.scoped_at.is_some());
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_undetermined_delivery_must_say_why(pool: PgPool) {
        // Invariant 6 wants the DLQ LEGIBLE, not merely visible. The service refuses before the
        // CHECK does, so the caller gets an answer instead of a 500.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        let err = record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::Undetermined,
                reason: None,
            },
        )
        .await
        .expect_err("undetermined without a reason must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_undetermined_delivery_stays_visible_and_never_becomes_out_of_scope_silently(
        pool: PgPool,
    ) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::Undetermined,
                reason: Some("enrichment failed: 502 from GitHub".into()),
            },
        )
        .await
        .expect("record undetermined");

        // It is on the DLQ read, and it says what stopped it.
        let stuck = list_undetermined(&pool, admin, sub)
            .await
            .expect("dlq read");
        assert_eq!(stuck.len(), 1);
        assert_eq!(
            stuck[0].scope_reason.as_deref(),
            Some("enrichment failed: 502 from GitHub")
        );

        // And it is still surfaced for judgment — invariant 6 requires the DLQ reach the tick,
        // not be quietly held back until someone fixes the enrichment.
        assert!(stuck[0].status.is_surfaced_for_judgment());
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_judged_delivery_cannot_be_rescoped(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");
        record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Declined,
                rationale: "touched only test fixtures".into(),
                confidence: 0.7,
                invocation_id: None,
            },
        )
        .await
        .expect("dispose");

        let err = record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::OutOfScope,
                reason: Some("late".into()),
            },
        )
        .await
        .expect_err("re-scoping a judged delivery must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    // ── the judgment leg ────────────────────────────────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_decline_is_an_authored_event_carrying_reasoning_and_confidence(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (webhook_event, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");

        let judged = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Declined,
                rationale: "judged immaterial: touched only test fixtures".into(),
                confidence: 0.7,
                invocation_id: None,
            },
        )
        .await
        .expect("dispose");

        assert_eq!(judged.disposition, Some(Disposition::Declined));
        assert_eq!(judged.confidence, Some(0.7));
        assert_eq!(judged.decided_by_profile_id, Some(*admin));
        let judgment_event = judged
            .decided_by_event_id
            .expect("a disposition cites its act");

        // The decline is on the LEDGER, not merely in a column — that is what makes it citable
        // rather than a silent cursor bump.
        let row = sqlx::query!(
            r#"SELECT t.name AS "name!", e.payload AS "payload!"
                 FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE e.id = $1"#,
            judgment_event,
        )
        .fetch_one(&pool)
        .await
        .expect("load judgment event");
        assert_eq!(row.name, "subscription_delivery_disposed");
        assert_eq!(
            row.payload["rationale"],
            "judged immaterial: touched only test fixtures"
        );
        assert_eq!(row.payload["confidence"], 0.7);
        assert_eq!(row.payload["event_id"], serde_json::json!(webhook_event));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_human_disposes_with_no_invocation_envelope(pool: PgPool) {
        // The clause's load-bearing property is ACCOUNTABLE AND CITABLE, not enveloped.
        // kb_invocations.originating_cogmap_id is NOT NULL, so requiring an envelope would make
        // this verb unreachable for kb_contexts and kb_teams subscribers entirely.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");
        let judged = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Acted,
                rationale: "authored a decision citing this PR".into(),
                confidence: 0.9,
                invocation_id: None,
            },
        )
        .await
        .expect("a human disposition must be accepted without an envelope");
        assert!(judged.decided_by_invocation_id.is_none());
        assert_eq!(
            judged.decided_by_profile_id,
            Some(*admin),
            "the profile carries attribution"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_disposition_without_reasoning_is_refused(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");
        let err = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Declined,
                rationale: "   ".into(),
                confidence: 0.5,
                invocation_id: None,
            },
        )
        .await
        .expect_err("a decline with no reasoning is the silent cursor bump this prevents");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_pending_scope_delivery_cannot_be_judged(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        let err = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Acted,
                rationale: "premature".into(),
                confidence: 0.5,
                invocation_id: None,
            },
        )
        .await
        .expect_err("judging an unscoped delivery must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_out_of_scope_delivery_cannot_be_judged(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::OutOfScope,
                reason: Some("no CODEOWNERS path hit".into()),
            },
        )
        .await
        .expect("scope");
        let err = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Declined,
                rationale: "nothing to decline".into(),
                confidence: 0.5,
                invocation_id: None,
            },
        )
        .await
        .expect_err("out_of_scope was determined not to touch this subscriber");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    // ── awareness is terminal, not backlog ──────────────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_awareness_only_subscriber_is_terminal_at_in_scope(pool: PgPool) {
        // A kb_contexts subscriber has no telos, no steward and no envelope. Its undisposed
        // delivery is a record of awareness, and the read surface returns it as an ordinary row
        // — there is no verb on this module that would report it as outstanding work.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");

        let rows = list_for_subscription(&pool, admin, sub, 50, 0)
            .await
            .expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, DeliveryStatus::InScope);
        assert!(
            rows[0].disposition.is_none(),
            "undisposed is terminal here, not pending"
        );
    }

    // ── C11: the routing is readable by the routed-to ───────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_subscriber_can_read_what_was_routed_to_it(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        for _ in 0..3 {
            intake_service::receive_webhook(
                &pool,
                conn,
                "pull_request",
                &github_pr_payload(GITHUB_REPO),
            )
            .await
            .expect("receive webhook");
        }
        let rows = list_for_subscription(&pool, admin, sub, 50, 0)
            .await
            .expect("list");
        assert_eq!(
            rows.len(),
            3,
            "every routed event is readable by the party it was routed to"
        );
        // Newest first, by uuidv7 id — byte order is time order.
        assert!(rows[0].id > rows[1].id && rows[1].id > rows[2].id);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_stranger_cannot_read_another_teams_deliveries(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        deliver_one(&pool, conn, sub, admin).await;

        // A profile that manages nothing. The delivery's readability is exactly its
        // subscription's, so this is refused by the same gate that guards the declaration.
        let stranger = seed_plain_profile(&pool).await;
        let err = list_for_subscription(&pool, stranger, sub, 50, 0)
            .await
            .expect_err("a stranger must not read another team's deliveries");
        assert!(
            matches!(err, ApiError::Forbidden | ApiError::NotFound(_)),
            "got {err:?}"
        );
    }

    // ── C12: a silent declaration vs a quiet source ─────────────────────────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn liveness_reports_matching_once_a_delivery_exists(pool: PgPool) {
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        deliver_one(&pool, conn, sub, admin).await;
        let live = declaration_liveness(&pool, admin, sub)
            .await
            .expect("liveness");
        assert_eq!(live, DeclarationLiveness::Matching { delivery_count: 1 });
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn liveness_distinguishes_a_wrong_selector_from_a_quiet_source(pool: PgPool) {
        // THE case the clause exists for. Payloads arrived on the connection; this declaration
        // matched none of them. Zero deliveries must NOT read as "the repo has been quiet".
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;
        let typo = create_subscription(
            &pool,
            admin,
            "kb_contexts",
            ctx,
            team,
            conn,
            SubscriptionSelector::GitHubRepository {
                repo: "acme/tempr".into(), // the typo
                event_types: vec![],
            },
        )
        .await;

        // A connection with no credential was never going to receive anything, and the read
        // says so rather than blaming the source. (Caught by this test failing the first time it
        // ran, which is the discriminator doing its job.)
        assert_eq!(
            declaration_liveness(&pool, admin, typo)
                .await
                .expect("liveness"),
            DeclarationLiveness::ConnectionNotAuthenticated,
            "an unauthenticated connection must not read as a quiet source"
        );

        // With a credential and nothing received, the honest answer is "the source has been quiet".
        attach_stub_credential(&pool, conn).await;
        assert_eq!(
            declaration_liveness(&pool, admin, typo)
                .await
                .expect("liveness"),
            DeclarationLiveness::SourceQuiet,
            "authenticated and silent means the source has been quiet"
        );

        intake_service::receive_webhook(
            &pool,
            conn,
            "pull_request",
            &github_pr_payload(GITHUB_REPO),
        )
        .await
        .expect("receive webhook");

        // Now two payloads-worth of evidence says the selector is the problem.
        assert_eq!(
            declaration_liveness(&pool, admin, typo)
                .await
                .expect("liveness"),
            DeclarationLiveness::SelectorMatchesNothing {
                events_on_connection: 1
            },
            "a live source with zero deliveries means the SELECTOR matched nothing"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn liveness_reports_revoked_before_anything_else(pool: PgPool) {
        let (admin, _t, _c, _conn, sub) = seed_world(&pool).await;
        crate::services::subscription_service::revoke(&pool, admin, sub)
            .await
            .expect("revoke");
        let live = declaration_liveness(&pool, admin, sub)
            .await
            .expect("liveness");
        assert!(
            matches!(live, DeclarationLiveness::Revoked { .. }),
            "revocation explains everything downstream of it; got {live:?}"
        );
    }

    // ── review findings: the guards must hold, and mean what they say ──────

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_judged_undetermined_delivery_leaves_the_dlq(pool: PgPool) {
        // `undetermined` admits judgment on purpose, so a steward can decline an enrichment
        // failure on its merits. Once judged the delivery is RESOLVED, and a "what is stuck?"
        // sweep that keeps returning it is a queue that can only grow. The status stays
        // `undetermined` — that is still the true statement about what enrichment concluded.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::Undetermined,
                reason: Some("enrichment failed: 502".into()),
            },
        )
        .await
        .expect("scope");
        assert_eq!(
            list_undetermined(&pool, admin, sub)
                .await
                .expect("dlq")
                .len(),
            1
        );

        let judged = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Declined,
                rationale: "judged immaterial despite the failed enrichment".into(),
                confidence: 0.4,
                invocation_id: None,
            },
        )
        .await
        .expect("an undetermined delivery is dispositionable");

        assert!(
            list_undetermined(&pool, admin, sub)
                .await
                .expect("dlq")
                .is_empty(),
            "a judged delivery has left the DLQ"
        );
        assert_eq!(
            judged.status,
            DeliveryStatus::Undetermined,
            "the status still records what enrichment concluded"
        );
        assert_eq!(
            judged.scope_reason.as_deref(),
            Some("enrichment failed: 502")
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn liveness_does_not_blame_the_selector_for_traffic_that_predates_the_declaration(
        pool: PgPool,
    ) {
        // The delivery count can only include events from after the subscription existed, so an
        // all-time event count compares two different windows and reports SelectorMatchesNothing
        // on a brand-new declaration against an established connection. The clause exists to stop
        // a maintainer misreading silence; that would manufacture the opposite misreading.
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;
        attach_stub_credential(&pool, conn).await;

        // History on the connection BEFORE any declaration exists.
        intake_service::receive_webhook(
            &pool,
            conn,
            "pull_request",
            &github_pr_payload(GITHUB_REPO),
        )
        .await
        .expect("historical webhook");

        let fresh = create_subscription(
            &pool,
            admin,
            "kb_contexts",
            ctx,
            team,
            conn,
            SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec![],
            },
        )
        .await;

        assert_eq!(
            declaration_liveness(&pool, admin, fresh)
                .await
                .expect("liveness"),
            DeclarationLiveness::SourceQuiet,
            "nothing has arrived SINCE this declaration; its selector is not the problem"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_scope_cannot_move_underneath_a_judgment(pool: PgPool) {
        // The `undetermined` direction is the one the schema does NOT rescue: every CHECK permits
        // it, so without the `disposition IS NULL` predicate the row would end up carrying a
        // judgment made against `in_scope` while reading `undetermined`.
        //
        // WHAT THIS DOES NOT WITNESS: the sequential path was already guarded by the pre-check, so
        // this test passes against the pre-fix code too. It is a regression guard, not evidence for
        // the concurrency fix — the interleaving that fix addresses (read on the pool, judge, then
        // write) has no deterministic test here, and the `disposition IS NULL` predicate plus the
        // `FOR UPDATE` in `record_disposition` are verified by construction rather than by
        // execution. Stated rather than left to be inferred from a green run.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");
        record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Acted,
                rationale: "authored against it".into(),
                confidence: 0.9,
                invocation_id: None,
            },
        )
        .await
        .expect("dispose");

        let err = record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::Undetermined,
                reason: Some("late enrichment failure".into()),
            },
        )
        .await
        .expect_err("re-scoping a judged delivery must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");

        let after = get(&pool, d.id).await.expect("reload");
        assert_eq!(
            after.status,
            DeliveryStatus::InScope,
            "the judged scope is intact"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_envelope_is_refused_for_a_subscriber_that_cannot_act_under_one(pool: PgPool) {
        // A kb_contexts subscriber has no telos. An envelope offered for it is incoherent, not
        // merely unverified — and the column's whole argument is that a citation is accountable.
        let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
        let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
        record_scope(
            &pool,
            admin,
            d.id,
            &RecordScopeRequest {
                status: DeliveryStatus::InScope,
                reason: None,
            },
        )
        .await
        .expect("scope");

        let err = record_disposition(
            &pool,
            admin,
            d.id,
            &RecordDispositionRequest {
                disposition: Disposition::Declined,
                rationale: "borrowed someone else's envelope".into(),
                confidence: 0.5,
                invocation_id: Some(Uuid::now_v7()),
            },
        )
        .await
        .expect_err("a context subscriber cannot cite an invocation envelope");
        let ApiError::BadRequest(msg) = err else {
            panic!("expected BadRequest, got {err:?}")
        };
        assert!(
            msg.contains("kb_cogmaps"),
            "must say which subscriber kind can: {msg}"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_refusal_message_is_not_mangled_by_source_wrapping(pool: PgPool) {
        // `cargo fmt` collapses a wrapped string literal without `\` continuations into one line,
        // baking the source indentation into the message. This message exists to be READ by a
        // maintainer chasing a stale registration record, so the mangling defeats its purpose.
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;
        register_webhook_events(&pool, conn, &["push"]).await;

        let err = crate::services::subscription_service::create(
            &pool,
            admin,
            &temper_core::types::subscription::CreateSubscriptionRequest {
                subscriber_table: "kb_contexts".into(),
                subscriber_id: ctx,
                authoring_team_id: team,
                connection_id: conn,
                selector: SubscriptionSelector::GitHubRepository {
                    repo: GITHUB_REPO.into(),
                    event_types: vec!["pull_request".into()],
                },
            },
        )
        .await
        .expect_err("refused");
        let ApiError::BadRequest(msg) = err else {
            panic!("expected BadRequest, got {err:?}")
        };
        assert!(
            !msg.contains("  "),
            "no run of double spaces in a message meant to be read: {msg:?}"
        );
    }

    // ── the inert declaration is refused at create, not disclosed forever ───

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_selector_waiting_on_an_unregistered_event_kind_is_refused_at_create(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;
        register_webhook_events(&pool, conn, &["push"]).await;

        let err = crate::services::subscription_service::create(
            &pool,
            admin,
            &temper_core::types::subscription::CreateSubscriptionRequest {
                subscriber_table: "kb_contexts".into(),
                subscriber_id: ctx,
                authoring_team_id: team,
                connection_id: conn,
                selector: SubscriptionSelector::GitHubRepository {
                    repo: GITHUB_REPO.into(),
                    event_types: vec!["pull_request".into()],
                },
            },
        )
        .await
        .expect_err("a declaration that can never match must be refused at create");
        let ApiError::BadRequest(msg) = err else {
            panic!("expected BadRequest, got {err:?}")
        };
        // The refusal CITES WHAT IT COMPARED, so a maintainer who knows the registration record
        // is stale can see why it said no rather than guess.
        assert!(
            msg.contains("pull_request"),
            "must name what the selector waits for: {msg}"
        );
        assert!(
            msg.contains("push"),
            "must name what the connection receives: {msg}"
        );
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_unregistered_connection_does_not_trigger_the_refusal(pool: PgPool) {
        // An EMPTY webhook_events is the not-yet-ledger-capable state, not proof that the
        // connection receives nothing forever. Refusing on it would reject a subscription against
        // a connection whose registration simply has not landed — the false-refusal risk that
        // made "warn instead" a real alternative. The bar is proof, and this is not proof.
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        crate::services::subscription_service::create(
            &pool,
            admin,
            &temper_core::types::subscription::CreateSubscriptionRequest {
                subscriber_table: "kb_contexts".into(),
                subscriber_id: ctx,
                authoring_team_id: team,
                connection_id: conn,
                selector: SubscriptionSelector::GitHubRepository {
                    repo: GITHUB_REPO.into(),
                    event_types: vec!["pull_request".into()],
                },
            },
        )
        .await
        .expect("an unregistered connection proves nothing and must not refuse");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_overlapping_selector_is_admitted(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;
        register_webhook_events(&pool, conn, &["push", "pull_request"]).await;

        crate::services::subscription_service::create(
            &pool,
            admin,
            &temper_core::types::subscription::CreateSubscriptionRequest {
                subscriber_table: "kb_contexts".into(),
                subscriber_id: ctx,
                authoring_team_id: team,
                connection_id: conn,
                selector: SubscriptionSelector::GitHubRepository {
                    repo: GITHUB_REPO.into(),
                    event_types: vec!["pull_request".into()],
                },
            },
        )
        .await
        .expect("one overlapping event kind is enough for the declaration to be live");
    }
}
