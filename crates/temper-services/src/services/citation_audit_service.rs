//! Citation-audit write service — the one caller-facing check the HTTP surface owns on top of
//! `DbBackend::record_citation_audit` (Set 5, Task 8; spec
//! `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §4.1-4.2).
//!
//! CONFORM to `handlers::edges::assert` and its siblings: an authored write surface builds a
//! `DbBackend` and dispatches exactly one command, with no persistence in the handler. This
//! module exists ONLY because that dispatch needs one thing no surface can do unaided — refuse a
//! path/block mismatch — and the lookup it needs, `finding_of_block`, is `pub(crate)` to this
//! crate (`crate::authz`, `authz/mod.rs:39`), so the check cannot live in temper-api.
//!
//! **Not a second gate.** `DbBackend::record_citation_audit` holds the ONE
//! `authorize::<AuditAuthority>` call in the codebase (`backend/db_backend.rs:1960-1962`), and
//! this function never calls `authorize`. It calls `finding_of_block` a SECOND time, which is a
//! pure `SELECT resource_id FROM kb_content_blocks WHERE id = $1` lookup — not an authorization
//! decision. `audit_gate.rs:70-71` names both call sites (the backend command and this HTTP
//! surface) as the one spelling's two intended callers, so this is the anticipated second use,
//! not a drift.
//!
//! 404, never 400/403, on a mismatch — the same refusal dialect `AuditAuthority::denial` uses
//! (`audit_gate.rs:159-173`), so a caller cannot distinguish "you named the wrong finding in the
//! path" from "this finding cannot be audited by you". Both are existence-shaped leaks otherwise.

use sqlx::PgPool;
use uuid::Uuid;

use crate::authz::finding_of_block;
use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_workflow::operations::{Backend, RecordCitationAudit};

/// Resolve `cmd.block`'s owning finding, refuse a mismatch against `path_finding`, then dispatch
/// the command through `DbBackend` — which re-derives the same finding and independently
/// authorizes over it (see module doc: two lookups, one gate).
///
/// Returns the new `kb_citation_audits.id`.
pub async fn record_citation_audit(
    pool: &PgPool,
    profile_id: ProfileId,
    path_finding: ResourceId,
    cmd: RecordCitationAudit,
) -> ApiResult<Uuid> {
    // The transposition guard: `cmd.block` addresses its own finding, and only that finding may
    // be named in `path_finding` (`/api/resources/{id}/citation-audits`). Letting them disagree
    // would let a caller cite a finding it may read in the path while actually writing an audit
    // onto a block of a different finding.
    let resolved_finding = finding_of_block(pool, cmd.block).await?;
    if resolved_finding != path_finding {
        return Err(ApiError::NotFound);
    }

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .record_citation_audit(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(out.value)
}
