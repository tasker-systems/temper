//! Delivery lifecycle — the projection of a matched radius into per-subscriber rows, and the two
//! legs (scope, then judgment) walked over them. S2 chunk C of "external systems as subscribed
//! emitters".
//!
//! **The delivery row is a projection, not a second event.** One webhook receipt is one ledger
//! event, always (goal C2); the delivery rows are *outcomes of processing* that event. They are
//! written in Rust inside the intake transaction, which CONFORMS to the precedent the goal itself
//! cites: `region_materialized` writes its N member rows in the event's own transaction
//! (`temper_substrate::write::materialize`) while its `_project_*` half only stamps a watermark.
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

use crate::auth::HumanPrincipal;
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
    .ok_or_else(|| ApiError::NotFound(DELIVERY_REFUSAL.to_string()))?
    .into_delivery()
}

/// [`get`], gated through the delivery's own subscription.
pub async fn get_for_caller(
    pool: &PgPool,
    authed: &HumanPrincipal,
    id: Uuid,
) -> ApiResult<Delivery> {
    let delivery = get(pool, id).await?;
    authorize_through_subscription(pool, authed, &delivery).await?;
    Ok(delivery)
}

/// The refusal for a delivery the caller cannot see — absent, or behind a subscription the caller
/// may not act on. One sentence for both, so a caller probing delivery ids cannot tell them apart.
const DELIVERY_REFUSAL: &str = "delivery not found or not readable";

/// Gate a delivery-id act through the delivery's own subscription, refusing in the **delivery's**
/// voice. The subscription gate refuses with its own `SUBSCRIPTION_REFUSAL` (a `404`
/// indistinguishable from a missing subscription); passed through unchanged, that sentence would
/// differ from a missing delivery's [`DELIVERY_REFUSAL`] and reopen the existence oracle one level
/// down. So every `NotFound` from the gate is re-rendered as the missing-delivery refusal. Any other
/// error passes through.
async fn authorize_through_subscription(
    pool: &PgPool,
    authed: &HumanPrincipal,
    delivery: &Delivery,
) -> ApiResult<temper_core::types::subscription::Subscription> {
    subscription_service::get_for_caller(pool, authed, delivery.subscription_id)
        .await
        .map_err(|err| match err {
            ApiError::NotFound(_) => ApiError::NotFound(DELIVERY_REFUSAL.to_string()),
            other => other,
        })
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
    authed: &HumanPrincipal,
    subscription_id: Uuid,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<Delivery>> {
    subscription_service::get_for_caller(pool, authed, subscription_id).await?;
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
    authed: &HumanPrincipal,
    subscription_id: Uuid,
) -> ApiResult<Vec<Delivery>> {
    subscription_service::get_for_caller(pool, authed, subscription_id).await?;
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
    authed: &HumanPrincipal,
    subscription_id: Uuid,
) -> ApiResult<DeclarationLiveness> {
    let sub = subscription_service::get_for_caller(pool, authed, subscription_id).await?;

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
    authed: &HumanPrincipal,
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
    authorize_through_subscription(pool, authed, &existing).await?;

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
    authed: &HumanPrincipal,
    delivery_id: Uuid,
    req: &RecordDispositionRequest,
) -> ApiResult<Delivery> {
    let caller = ProfileId::from(authed.profile().id);
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
    let sub = authorize_through_subscription(pool, authed, &existing).await?;

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
    .ok_or_else(|| ApiError::NotFound(DELIVERY_REFUSAL.to_string()))?;

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
mod tests;
