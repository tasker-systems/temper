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
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::admin_ledger_service::{self, AdminLedgerEntry};
use temper_services::state::AppState;
use temper_substrate::payloads::{AnchorTable, RefTarget};

use crate::middleware::auth::AuthUser;

/// Page size when the caller does not ask for one, and the ceiling when they ask for too much.
/// Clamped rather than rejected: a caller asking for more than the cap wants "as much as you
/// will give me", and a 400 there teaches nothing.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// `GET /api/admin/ledger` — exactly one axis, never both.
#[derive(Debug, Deserialize)]
pub struct LedgerQuery {
    /// Subject axis, table half: `kb_contexts`, `kb_cogmaps`, … Parsed through `AnchorTable`'s
    /// own serde so the accepted set is the enum, not a second list that can drift from it.
    pub subject_kind: Option<String>,
    /// Subject axis, id half.
    pub subject_id: Option<Uuid>,
    /// Actor axis: whose acts to read. Self-gating — you may always read your own history.
    pub actor: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Entries plus the epoch, always together.
///
/// The epoch is what stops an empty `entries` from lying. Admin history *begins* at the epoch —
/// acts before it genuinely happened but no writer recorded them (spec §8) — so an empty list
/// with an epoch means "nothing since T", never "nothing ever". Carrying it on every response is
/// the whole reason there is no standalone epoch route: `ledger_epoch` takes no caller and has no
/// gate, so the only safe place to surface it is inside a response the service has already gated.
#[derive(Debug, Serialize)]
pub struct AdminLedgerResponse {
    pub entries: Vec<AdminLedgerEntry>,
    pub epoch: Option<DateTime<Utc>>,
}

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

fn resolve_axis(q: &LedgerQuery) -> ApiResult<Axis> {
    let has_subject = q.subject_kind.is_some() || q.subject_id.is_some();
    let has_actor = q.actor.is_some();

    match (has_subject, has_actor) {
        (true, true) => Err(ApiError::BadRequest(
            "pass either subject_kind+subject_id or actor, not both".to_string(),
        )),
        (false, false) => Err(ApiError::BadRequest(
            "pass either subject_kind+subject_id or actor".to_string(),
        )),
        (false, true) => Ok(Axis::Actor(ProfileId::from(
            q.actor.expect("has_actor checked"),
        ))),
        (true, false) => {
            // Half a subject is not a subject. Naming the missing half beats a generic 400.
            let (Some(kind), Some(id)) = (q.subject_kind.as_deref(), q.subject_id) else {
                return Err(ApiError::BadRequest(
                    "subject_kind and subject_id must be given together".to_string(),
                ));
            };
            Ok(Axis::Subject(RefTarget {
                kind: parse_anchor_table(kind)?,
                id,
            }))
        }
    }
}

/// Parse a table name into `AnchorTable` via the enum's own serde renames, which already spell
/// the table names exactly. A hand-written match here would be a second copy of a bounded set,
/// free to drift from the enum the moment a variant is added.
fn parse_anchor_table(kind: &str) -> ApiResult<AnchorTable> {
    serde_json::from_value::<AnchorTable>(serde_json::Value::String(kind.to_string()))
        .map_err(|_| ApiError::BadRequest(format!("unknown subject_kind '{kind}'")))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<LedgerQuery>,
) -> ApiResult<Json<AdminLedgerResponse>> {
    let caller = ProfileId::from(auth.0.profile.id);
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

    Ok(Json(AdminLedgerResponse { entries, epoch }))
}
