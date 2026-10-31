//! The admin ledger's read surface, documented under the `Admin Ledger` tag. Not in the admin
//! route group (`routes/admin.rs`) despite its path: the gate below admits the actor axis and
//! per-family readers, not only admins.
//!
//! **Authorization lives in the service, not here.** `admin_ledger_service` gates both axes
//! itself, and it does so by *dispatching per act family* rather than with a single prelude:
//! `readable_event_types` computes what this caller may read about this subject and turns that
//! into the query's `t.name = ANY($1)` bind. A gate here could not do that — no event type is
//! known until rows come back. See the service's own note.
//!
//! Deny is **404, never 403** (`list_by_subject:100-104`): a 403 would confirm the ledger has
//! something to hide about this subject, which is itself the disclosure. The contract states the
//! 404, because the 404 protects the subject, not the door.

use axum::extract::{Query, State};
use axum::Json;

use temper_core::types::admin::{AdminLedgerQuery, AdminLedgerResponse};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::admin_ledger_service;
use temper_services::state::AppState;
use temper_substrate::payloads::RefTarget;

use crate::middleware::auth::AuthUser;

/// Page size when the caller does not ask for one, and the ceiling when they ask for too much.
/// Clamped rather than rejected: a caller asking for more than the cap wants "as much as you
/// will give me", and a 400 there teaches nothing.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// Resolve the requested axis, refusing ambiguity rather than silently preferring one.
///
/// Two axes that answer different questions ("what was done TO this subject" vs "what did this
/// actor DO") and gate differently — subject-gated vs self-gating. Picking one for the caller
/// when they named both would answer a question they did not ask, under a gate they did not
/// expect.
enum Axis {
    Subject(RefTarget),
    Actor(ProfileId),
}

fn resolve_axis(q: &AdminLedgerQuery) -> ApiResult<Axis> {
    match (q.subject.as_deref(), q.actor) {
        (Some(_), Some(_)) => Err(ApiError::BadRequest(
            "pass either subject or actor, not both".to_string(),
        )),
        (None, None) => Err(ApiError::BadRequest(
            "pass either subject ('<kind>:<uuid>') or actor".to_string(),
        )),
        // The one place the `<kind>:<uuid>` spelling is understood, shared with temper-mcp.
        (Some(spec), None) => Ok(Axis::Subject(admin_ledger_service::parse_subject_spec(
            spec,
        )?)),
        (None, Some(actor)) => Ok(Axis::Actor(ProfileId::from(actor))),
    }
}

#[utoipa::path(
    get,
    operation_id = "list_admin_ledger",
    summary = "Read the admin ledger",
    description = "Returns a page of recorded administrative acts, newest first, on exactly one axis. `subject` (`<kind>:<uuid>`, e.g. `kb_resources:<uuid>`) returns the acts performed on that subject, limited to the act families the caller may read about it: a system admin reads all of them, a caller who may administer grants on the subject reads its grant acts, and an owner or maintainer of a team (`kb_teams:<uuid>`) reads that team's membership changes and the linking or detaching of its child teams (on the gating team, only a system admin does). `actor` returns the acts a profile performed: any caller may read their own, and only a system admin may read another's. A read the caller is not allowed is answered 404, not 403, so a refusal reveals nothing about the subject. `epoch` is when recording began; acts before it were not recorded. `limit` defaults to 50 and is capped at 200.",
    path = "/api/admin/ledger",
    tag = "Admin Ledger",
    params(AdminLedgerQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "A page of ledger entries, with the ledger epoch", body = AdminLedgerResponse),
        (status = 400, description = "Neither or both of `subject` and `actor` were given, or `subject` is malformed. A query value that does not parse (e.g. a non-UUID `actor`) is a plain-text rejection, not an ErrorBody", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = ErrorBody),
        (status = 404, description = "The caller may read nothing on this axis: no readable act family for the subject, or another profile's acts without being a system admin", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<AdminLedgerQuery>,
) -> ApiResult<Json<AdminLedgerResponse>> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = q.offset.unwrap_or(0).max(0);

    let entries = match resolve_axis(&q)? {
        Axis::Subject(subject) => {
            admin_ledger_service::list_by_subject(&state.pool, &auth.0, subject, limit, offset)
                .await?
        }
        Axis::Actor(actor) => {
            admin_ledger_service::list_by_actor(&state.pool, &auth.0, actor, limit, offset).await?
        }
    };

    // Only reached once the service has authorized the read above.
    let epoch = admin_ledger_service::ledger_epoch(&state.pool).await?;

    // The projection is shared with temper-mcp so the two surfaces cannot answer different
    // shapes to the same question.
    Ok(Json(admin_ledger_service::to_wire_page(entries, epoch)?))
}
