//! The admin ledger's read surface. Out of the OpenAPI contract (plain `.route()` mounting),
//! like `/api/access/admin/*` and `/api/machine-clients*` — its path is on the allowlist in
//! `.github/scripts/check-openapi-routes.sh`.
//!
//! **Authorization lives in the service, not here.** `admin_ledger_service` gates both axes
//! itself, and it does so by *dispatching per act family* rather than with a single prelude:
//! `readable_event_types` computes what this caller may read about this subject and turns that
//! into the query's `t.name = ANY($1)` bind. A gate here could not do that — no event type is
//! known until rows come back. See the service's own note.
//!
//! Deny is **404, never 403** (`list_by_subject:100-104`): a 403 would confirm the ledger has
//! something to hide about this subject, which is itself the disclosure.

use axum::extract::{Query, State};
use axum::Json;

use temper_core::types::admin::{AdminLedgerQuery, AdminLedgerResponse};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult};
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

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<AdminLedgerQuery>,
) -> ApiResult<Json<AdminLedgerResponse>> {
    let caller = ProfileId::from(auth.0.profile().id);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = q.offset.unwrap_or(0).max(0);

    let entries = match resolve_axis(&q)? {
        Axis::Subject(subject) => {
            admin_ledger_service::list_by_subject(&state.pool, caller, subject, limit, offset)
                .await?
        }
        Axis::Actor(actor) => {
            admin_ledger_service::list_by_actor(&state.pool, caller, actor, limit, offset).await?
        }
    };

    // Only reached once the service has authorized the read above.
    let epoch = admin_ledger_service::ledger_epoch(&state.pool).await?;

    // The projection is shared with temper-mcp so the two surfaces cannot answer different
    // shapes to the same question.
    Ok(Json(admin_ledger_service::to_wire_page(entries, epoch)?))
}
