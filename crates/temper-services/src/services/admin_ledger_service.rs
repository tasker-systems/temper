//! The admin ledger's read surface.
//!
//! Admin events are NULL-anchored (spec 2026-07-16 §4) — the cognition firewall. That firewall
//! is structural: every region producer, `steward_ingest_delta`, materialize attribution, and
//! `latest_event_id_for_context` scope by `producing_anchor_table`, so a both-NULL event is
//! invisible to all of them. It is equally invisible to every *reader*, which is why identity
//! lives in `kb_events."references"` (GIN-indexed, and consulted by no cognition reader).
//!
//! Two axes:
//!   - by subject → `references @> …` (idx_kb_events_references, jsonb_path_ops)
//!   - by actor   → `kb_profiles.id = …`, reached through `kb_events → kb_entities → kb_profiles`
//!
//! **The actor axis is a two-hop join, not a direct `emitter_entity_id` lookup** — this header
//! used to say the latter, which reads as though the actor id were the index condition on
//! `idx_kb_events_emitter`. It is not: a profile owns many entities
//! (`kb_entities_profile_id_name_key` is UNIQUE on `(profile_id, name)`), so the actor predicate
//! lands on `kb_profiles.id` one hop past the emitter. `idx_kb_events_emitter` still serves the
//! **join** (`Index Cond: emitter_entity_id = en.id`), which is why the distinction is easy to
//! miss. Whether the entity→profile step wants its own index is a real question, but it is a
//! question about production data volume, and the dev database is far too small to answer it —
//! every row estimate there is 1, so the plan shape proves nothing either way.
//!
//! This surface ships BEFORE any writer, deliberately (spec §5): the NULL anchor that firewalls
//! admin events from cognition also hides them from every reader, so the read path had to be
//! designed first or the writers would target a query shape nobody had proved.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_substrate::payloads::{EventRef, RefTarget};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::access_service;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminLedgerEntry {
    pub event_id: Uuid,
    pub event_type: String,
    pub actor_profile_id: Uuid,
    pub actor_handle: String,
    pub occurred_at: DateTime<Utc>,
    pub payload: serde_json::Value,
    pub references: Vec<EventRef>,
    pub correlation_id: Option<Uuid>,
}

/// Admin event types. The ledger read surface returns ONLY these — never cognition events that
/// happen to share the NULL-anchor bucket (`lens_created` is already in it). Discriminating by
/// anchor nullity would silently absorb system-config events; discriminate by type.
///
/// `admin_ledger_opened` has no `kb_event_types` row until the epoch marker migration ships. That
/// is harmless: `= ANY($1)` simply matches nothing and `ledger_epoch` returns `None`.
const ADMIN_EVENT_TYPES: &[&str] = &["admin_ledger_opened", "grant_created", "grant_revoked"];

/// The §5 table, evaluated for one subject. Returns the event types `caller` may read about
/// `subject` — empty means "nothing", which the caller turns into 404.
///
/// **This IS the gate.** There is no `gate(pool, caller)` prelude, and the difference is
/// structural rather than editorial: a gate that runs before the query cannot dispatch on event
/// type, because no event type is known until rows come back — and dispatching per event type is
/// the whole of §5. So it is inverted: compute what the caller may read, then ask only for that.
/// The subject is a *parameter*, so the answer is fixed before the query and becomes its
/// `t.name = ANY($1)` bind.
///
/// One gate call per act family, NOT one per row: per-row gating would be an N+1 (two queries per
/// row) AND would silently break LIMIT/OFFSET — filtering after the window means page 2 is not the
/// second 50 readable rows, it is whatever survived of the second 50 raw rows.
async fn readable_event_types(
    pool: &PgPool,
    caller: ProfileId,
    subject: RefTarget,
) -> ApiResult<Vec<&'static str>> {
    // Admin reads everything; one query, and the common admin path stops here.
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(ADMIN_EVENT_TYPES.to_vec());
    }

    let mut readable = Vec::new();

    // grant_created / grant_revoked → mirrors access_service::can_administer_grant, by CALLING it.
    // (is_system_admin is already OR-ed inside it; we short-circuited above, so this is the
    // can_grant arm doing the work.)
    if access_service::can_administer_grant(pool, caller, subject.kind.as_str(), subject.id).await?
    {
        readable.push("grant_created");
        readable.push("grant_revoked");
    }

    // admin_ledger_opened → the epoch marker. is_system_admin only; handled by the arm above.
    // Machine/connection acts → machine_authz::authorize(owner_team). NOT REACHED IN THIS TASK:
    //   no such event type exists until step 5 of the spec's §9, and ADMIN_EVENT_TYPES does not
    //   list one. When one is added, it gets an arm HERE, and the default below keeps it
    //   admin-only until someone does.
    //
    // Default: absent from this fn ⇒ admin-only ⇒ fail closed. The default arm is expressed as
    // ABSENCE from the returned set rather than as a match arm nobody wrote.
    Ok(readable)
}

/// "Who was granted what on this subject, and when?"
pub async fn list_by_subject(
    pool: &PgPool,
    caller: ProfileId,
    subject: RefTarget,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    let types = readable_event_types(pool, caller, subject).await?;
    if types.is_empty() {
        // Reads deny with 404, not 403 — the deny-split invariant. A 403 would confirm the
        // ledger has something to hide about this subject.
        return Err(ApiError::NotFound);
    }

    // The `rel` is pinned to `subject` deliberately. `[{"target": …}]` alone would also match a
    // `principal` or `touches` reference to the same id — "every act performed FOR this team"
    // silently answering a query that says "performed ON it". jsonb_path_ops containment indexes
    // the fuller object just as well (verified: Bitmap Index Scan under enable_seqscan=off).
    let probe = serde_json::to_value([EventRef::subject(subject)])
        .map_err(|e| ApiError::Internal(format!("subject probe: {e}")))?;
    fetch(pool, &types, Some(probe), None, limit, offset).await
}

/// "What did this admin do?"
///
/// The actor axis is **self-gating** (spec §11.1b, decided 2026-07-16): you may always read the
/// record of acts you performed. Losing a capability, a role, or ownership of a subject does not
/// take your own history from you — only losing system access does, because then you are not a
/// reader at all.
///
/// Deliberately NOT subject-gated. The defect that motivated this whole spec is
/// `kb_access_grants` destroying `granted_by_profile_id` on upsert; a ledger that restores
/// authorship and then hides it from its author would be a poor trade. Probed live: §5's
/// `can_grant` arm carries ZERO of prod's 5 real grants, so a subject-gate here would today mean
/// "admins only" — and ownership is mutable (`rehome`/`reassign` ship), so the demoted actor is
/// reachable by ordinary usage, not just by demotion.
pub async fn list_by_actor(
    pool: &PgPool,
    caller: ProfileId,
    actor: ProfileId,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    // THE ONLY GATE — not defense in depth. Both surfaces authenticate and hand `caller`
    // straight here: `handlers::admin_ledger::list` mounts on a plain `.route()` with no
    // prelude, and `tools::admin_ledger` calls `require_profile()` and no more. Neither gates
    // upstream. Do not relax this on the theory that a layer above repeats it — nothing does.
    //
    // How much this check actually excludes is an OPERATIONAL setting, not a property of this
    // code — do not reason about it from a value read at some past moment. Treat the real gate on
    // the actor axis as the caller-is-the-actor test below, and on the subject axis as
    // `readable_event_types`; both hold regardless of how the instance is configured. That is
    // intended: reading your own authorship is not an admin act.
    if !access_service::has_system_access(pool, caller).await? {
        return Err(ApiError::NotFound);
    }

    // Reading someone else's history is an audit, and audits are admin-only.
    if caller != actor && !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::NotFound);
    }

    // No per-subject gate: that is the decision. The full catalogue is correct here precisely
    // because the axis is the caller's own authorship (or an admin's audit).
    fetch(
        pool,
        ADMIN_EVENT_TYPES,
        None,
        Some(actor.uuid()),
        limit,
        offset,
    )
    .await
}

/// The epoch: admin history begins here. NOT a backfill marker — everything before this is
/// genuinely unrecorded (spec §8), and the surface must say so rather than imply absence.
pub async fn ledger_epoch(pool: &PgPool) -> ApiResult<Option<DateTime<Utc>>> {
    Ok(sqlx::query_scalar!(
        "SELECT e.occurred_at FROM kb_events e
           JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'admin_ledger_opened'
          ORDER BY e.occurred_at ASC LIMIT 1"
    )
    .fetch_optional(pool)
    .await?)
}

/// One row of the ledger join, as the driver returns it.
type LedgerRow = (
    Uuid,
    String,
    Uuid,
    String,
    DateTime<Utc>,
    serde_json::Value,
    serde_json::Value,
    Option<Uuid>,
);

async fn fetch(
    pool: &PgPool,
    // The gate's output, not the whole catalogue: `readable_event_types` decided this.
    types: &[&str],
    subject_probe: Option<serde_json::Value>,
    actor: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    // Runtime `query_as` rather than the `query_as!` macro: the two axes select different
    // predicates over one statement (`$2::jsonb IS NULL OR …`, `$3::uuid IS NULL OR …`), which is
    // the dynamic-predicate case the `search_service` precedent covers. The columns are fixed and
    // the binds are parameters — nothing is interpolated.
    let rows = sqlx::query_as::<_, LedgerRow>(
        r#"SELECT e.id, t.name, p.id, p.handle, e.occurred_at, e.payload, e."references", e.correlation_id
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             JOIN kb_entities   en ON en.id = e.emitter_entity_id
             JOIN kb_profiles    p ON p.id = en.profile_id
            WHERE t.name = ANY($1)
              AND ($2::jsonb IS NULL OR e."references" @> $2::jsonb)
              AND ($3::uuid  IS NULL OR p.id = $3::uuid)
            ORDER BY e.occurred_at DESC, e.id DESC
            LIMIT $4 OFFSET $5"#,
    )
    // The authorized set, NOT ADMIN_EVENT_TYPES. Binding the catalogue here would make the gate
    // decorative — it would compute a type set and then query for every type anyway.
    .bind(types)
    .bind(subject_probe)
    .bind(actor)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(
            |(
                event_id,
                event_type,
                actor_profile_id,
                actor_handle,
                occurred_at,
                payload,
                refs,
                correlation_id,
            )| {
                Ok(AdminLedgerEntry {
                    event_id,
                    event_type,
                    actor_profile_id,
                    actor_handle,
                    occurred_at,
                    payload,
                    references: serde_json::from_value(refs).map_err(|e| {
                        ApiError::Internal(format!("malformed references on {event_id}: {e}"))
                    })?,
                    correlation_id,
                })
            },
        )
        .collect()
}

/// Project a page onto the shared wire type in `temper_core::types::admin`.
///
/// **Both surfaces call this one function.** temper-api and temper-mcp each expose this read, and
/// a conversion written twice is a wire contract that can answer two different shapes to the same
/// question — the exact drift class the admin-event-sink arc exists to close. It lives here rather
/// than in either surface because this crate is the one both already depend on.
///
/// The service type and the wire type are structurally identical but deliberately distinct:
/// temper-client must deserialize the wire type and cannot depend on temper-substrate (which
/// pulls `temper-ingest(embed)`, and would link ort/ONNX into the HTTP client). See the note in
/// `temper_core::types::admin`; `admin_ledger_wire_parity_test` pins the two together.
///
/// `references` round-trips through JSON rather than being matched variant-by-variant — the
/// vocabulary is a bounded set the parity test already pins, so a hand-written match here would be
/// a third copy free to drift from both.
pub fn to_wire_page(
    entries: Vec<AdminLedgerEntry>,
    epoch: Option<DateTime<Utc>>,
) -> ApiResult<temper_core::types::admin::AdminLedgerResponse> {
    let entries = entries
        .into_iter()
        .map(|e| {
            let references = serde_json::to_value(&e.references)
                .and_then(serde_json::from_value)
                .map_err(|err| ApiError::Internal(format!("ledger refs: {err}")))?;

            Ok(temper_core::types::admin::AdminLedgerEntry {
                event_id: e.event_id,
                event_type: e.event_type,
                actor_profile_id: e.actor_profile_id,
                actor_handle: e.actor_handle,
                occurred_at: e.occurred_at,
                payload: e.payload,
                references,
                correlation_id: e.correlation_id,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;

    Ok(temper_core::types::admin::AdminLedgerResponse { entries, epoch })
}

/// Parse a table name into `AnchorTable` via the enum's own serde renames, which already spell the
/// table names exactly. Never a hand-written match: that would be a second copy of a bounded set,
/// free to drift from the enum the moment a variant is added.
///
/// Private: surfaces take the whole `<kind>:<uuid>` spelling via [`parse_subject_spec`], so the
/// kind half never crosses a boundary on its own.
fn parse_anchor_table(kind: &str) -> ApiResult<temper_substrate::payloads::AnchorTable> {
    serde_json::from_value(serde_json::Value::String(kind.to_string()))
        .map_err(|_| ApiError::BadRequest(format!("unknown subject kind '{kind}'")))
}

/// Parse the `<kind>:<uuid>` subject spelling that the CLI and MCP both accept.
///
/// Split from the RIGHT: the uuid half is fixed-shape, so a stray colon belongs to the kind half,
/// not the id. Lives here rather than in a surface because both surfaces take this spelling, and a
/// parser written twice is two grammars.
pub fn parse_subject_spec(spec: &str) -> ApiResult<RefTarget> {
    let (kind, id) = spec.rsplit_once(':').ok_or_else(|| {
        ApiError::BadRequest(format!(
            "subject must be '<kind>:<uuid>' (e.g. kb_resources:0199...), got '{spec}'"
        ))
    })?;

    let id = uuid::Uuid::parse_str(id)
        .map_err(|e| ApiError::BadRequest(format!("invalid subject id '{id}' in '{spec}': {e}")))?;

    Ok(RefTarget {
        kind: parse_anchor_table(kind)?,
        id,
    })
}
