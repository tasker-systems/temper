//! Delivery types — one row per (subscription, event) the radius matched, and the two-legged
//! lifecycle over it (S2 chunk C of "external systems as subscribed emitters", spec 2026-07-13).
//!
//! **Two lifecycles share this table.** `kb_invocations.originating_cogmap_id` is `NOT NULL`, so
//! only a `kb_cogmaps` subscriber can ever be acted for under an invocation envelope. A
//! `kb_contexts` or `kb_teams` subscriber has no telos, no steward and no envelope — it subscribed
//! in order to **be aware**, and its delivery is terminal at `in_scope`/`undetermined`. That
//! undisposed row is a record of awareness, never a backlog entry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The scope leg. A delivery is born [`Self::PendingScope`] because the coarse radius is
/// payload-only; enrichment (S4) resolves it.
///
/// [`Self::Undetermined`] is a **first-class terminal state, not an error code**. Goal invariant 6:
/// an enrichment that fails leaves the delivery visible and must never silently resolve to
/// `out_of_scope`. It is an ordinary variant precisely so no code path treats it as exceptional and
/// collapses it — and it always carries a reason, because visible-without-being-legible does not
/// distinguish *"I could not see whether it was"* from *"nothing was touched"*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    PendingScope,
    InScope,
    OutOfScope,
    Undetermined,
}

impl DeliveryStatus {
    /// The DDL spelling, exactly as `kb_subscription_deliveries.status`'s CHECK admits it.
    /// Deliberately not `serde_json::to_string`, which would yield a *quoted* value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PendingScope => "pending_scope",
            Self::InScope => "in_scope",
            Self::OutOfScope => "out_of_scope",
            Self::Undetermined => "undetermined",
        }
    }

    /// Whether a delivery in this state is one a judging subscriber's tick should see. `in_scope`
    /// is the obvious half; `undetermined` is the half that matters — invariant 6 requires the DLQ
    /// be surfaced too, not quietly held back until someone fixes the enrichment.
    pub fn is_surfaced_for_judgment(self) -> bool {
        matches!(self, Self::InScope | Self::Undetermined)
    }
}

/// The judgment leg. `None` on a delivery means *not judged*, which for an awareness-only
/// subscriber is the expected terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Judged material and authored against — the authored work cites the event.
    Acted,
    /// Judged immaterial, **with** reasoning and confidence. A decline is accountable and citable,
    /// not a silent cursor bump.
    Declined,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acted => "acted",
            Self::Declined => "declined",
        }
    }
}

/// One routed event, and what became of it for one declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub id: Uuid,
    pub subscription_id: Uuid,
    /// The `kb_events` row this delivery projects from. One webhook is one event; the delivery
    /// rows are outcomes of processing it, never a second event.
    pub event_id: Uuid,
    pub status: DeliveryStatus,
    /// Why the scope resolved as it did. Always present when `status` is
    /// [`DeliveryStatus::Undetermined`] — the schema refuses the row otherwise.
    pub scope_reason: Option<String>,
    pub scoped_at: Option<DateTime<Utc>>,
    pub disposition: Option<Disposition>,
    /// The authored judgment event. A disposition is an act on the ledger; `rationale` and
    /// `confidence` below are a queryable projection of that event's payload, not a second source
    /// of truth.
    pub decided_by_event_id: Option<Uuid>,
    /// Present only when an agent acted for a cogmap. `None` for a human disposition — see the
    /// module docs on why requiring it would scope judgment to one subscriber kind.
    pub decided_by_invocation_id: Option<Uuid>,
    pub decided_by_profile_id: Option<Uuid>,
    pub decided_at: Option<DateTime<Utc>>,
    pub rationale: Option<String>,
    pub confidence: Option<f64>,
    pub created: DateTime<Utc>,
}

/// Record the outcome of scoping a delivery. Enrichment (S4) is the caller; this ships now so the
/// state machine has a witness rather than a hole between intake and a chunk that does not exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordScopeRequest {
    pub status: DeliveryStatus,
    /// Required when `status` is [`DeliveryStatus::Undetermined`], and the service refuses without
    /// it rather than letting the database raise — a constraint violation surfaced as a 500 tells
    /// the caller nothing about invariant 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Author a judgment on a delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordDispositionRequest {
    pub disposition: Disposition,
    /// Why. Required by both the type and the schema — a disposition without reasoning is the
    /// silent cursor bump the delivery row exists to make impossible.
    pub rationale: String,
    /// The judgment's confidence in `[0,1]`.
    pub confidence: f64,
    /// The invocation this judgment was made under, when an agent is acting for a cogmap. `None`
    /// for a human disposition; the acting profile carries attribution instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<Uuid>,
}

/// What a subscriber can be told about a declaration that has produced no deliveries.
///
/// This is goal clause C12 (`a-silent-declaration-is-distinguishable-from-a-quiet-source`). Zero
/// deliveries is the same observation for "working correctly, nothing happened" and "silently
/// broken since the day you set it up", and the clause is a claim about **absence** — so no table
/// of things-that-happened answers it alone. The variants below are read off three facts that
/// already exist, with no write added to the intake path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeclarationLiveness {
    /// Deliveries exist. The declaration demonstrably works.
    Matching { delivery_count: i64 },
    /// Payloads reached the connection and this selector matched none of them. The overwhelmingly
    /// likely cause is the selector, and saying so is the whole point of the clause.
    SelectorMatchesNothing { events_on_connection: i64 },
    /// The connection is authenticated and simply has not received anything. Nothing is wrong.
    SourceQuiet,
    /// The connection holds no credential (`credential IS NULL` is the `needs_credential` birth
    /// state), so nothing was ever going to arrive for any subscription on it.
    ConnectionNotAuthenticated,
    /// The declaration was revoked. It stopped matching by design.
    Revoked { revoked_at: DateTime<Utc> },
}
