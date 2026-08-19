//! Citation-audit service — the write path's one caller-facing check on top of
//! `DbBackend::record_citation_audit`, and the attributed trail read beside it (Set 5, Task 8 and
//! the per-auditor collapse Beat 3; spec
//! `internal/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §4.1-4.2, §5.2).
//!
//! **Why the trail read lives here and not in `evidential_standing_service`.** That module's own
//! doc scopes it to *"the one live read over a finding's standing shape"* — a fixed-width aggregate
//! recomputed live. The trail is not a component of that shape; it is the ledger the shape
//! summarizes, at the `(block, source)` grain this module already owns on the write side. Putting
//! it there would falsify that module's doc and split the citation-audit surface across two files
//! for no gain; putting it here keeps `POST` and `GET` on the same path in the same module.
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
//!
//! # Why the READ 404s instead of returning `[]`
//!
//! The argument lives here rather than on the `temper-api` handler because it is about the PAIR of
//! endpoints, and because a handler doc is lifted verbatim into `openapi.json` and from there into
//! every generated client — the same reason `/evidence`'s leak-safety argument lives in
//! `evidential_standing_service`'s module doc and its handler carries none.
//!
//! Collections in this codebase generally return an empty array rather than erroring — the
//! `/provenance` sibling does exactly that (`resource_block_provenance` JOIN-filters through
//! `resources_readable_by` and yields an empty set for an unreadable resource). This one does not,
//! and the reason is its neighbour rather than its shape.
//!
//! `GET /api/resources/{id}/evidence` **404s** on an unreadable or absent finding, and
//! `evidential_standing_service`'s module doc calls that leak-safety: *"a denied and an absent
//! finding are indistinguishable."* If this endpoint answered `200 []` where `/evidence` answers
//! `404`, the pair of responses would carry information neither carries alone — the caller would
//! learn only that BOTH refuse, which is fine, but the moment a readable-with-no-audits finding also
//! answers `200 []` the two cases stop being separable **for the honest caller** while remaining
//! separable for the probing one through any future divergence. Rather than reason case-by-case
//! about which diffs leak, this read takes the refusal dialect its own path already speaks:
//! `AuditAuthority` renders **both** of its denial arms as `NotFound` precisely so the `POST` never
//! becomes an existence oracle (`crate::authz::audit_gate`), and [`record_citation_audit`] answers a
//! path/block mismatch the same way. A `GET` on that path that refused in a different dialect than
//! the `POST` would be the drift.
//!
//! So: **404 when the finding is not readable (or does not exist), `200 []` only when it IS readable
//! and genuinely carries no audits.** [`list_citation_audits`] asks readability through
//! `readback::is_resource_visible` — the same predicate, not a second spelling of it — before running
//! the trail SQL, whose own in-SQL gate makes all three empty cases identical at the database layer.

use sqlx::PgPool;
use uuid::Uuid;

use crate::authz::{finding_of_block, FINDING_REFUSAL};
use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult};
use temper_core::error::TemperError;
use temper_core::types::citation_audit::CitationAuditRow;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_substrate::readback;
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
        // Same string as every other finding-shaped refusal — a transposition attempt must not be
        // distinguishable from an unknown block or an unreadable finding.
        return Err(ApiError::NotFound(FINDING_REFUSAL.to_string()));
    }

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .record_citation_audit(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(out.value)
}

/// Read `finding`'s citation-audit trail — one row per audit, each naming its auditor.
///
/// **Two calls, one predicate, and the split is the whole design.** The trail SQL
/// (`resource_citation_audit_trail`) carries the access gate INSIDE it, so it answers *"unreadable",
/// "absent"* and *"readable, no audits"* identically with zero rows — that indistinguishability is
/// what keeps it leak-safe. But a collection surface must tell the honest caller's empty trail apart
/// from a refusal, so readability is asked SEPARATELY, first, via
/// [`readback::is_resource_visible`] — the Rust-callable spelling of the very same predicate
/// (`resources_readable_by('profile', p)` delegates its profile arm to `resources_visible_to`,
/// `migrations/20260712000010_context_read_predicates.sql:419`), and the same call
/// `AuditAuthority`'s readability arm makes for the POST on this path
/// (`crate::authz::audit_gate`). GET and POST therefore refuse on identical grounds, and nothing
/// here restates a gate.
///
/// **404 on unreadable, `[]` on readable-and-empty — deliberately NOT the collection default.** See
/// the module doc for the argument; the short form is that returning 200 where the sibling
/// `/evidence` returns 404 puts a readability oracle one endpoint-diff away, and the Set 5 design
/// spends the whole `AuditAuthority` denial dialect avoiding exactly that.
///
/// Rows arrive already ordered (most recent first) and already restricted to the audits the standing
/// axes summed; this function only maps the substrate-local record to the wire type.
pub async fn list_citation_audits(
    pool: &PgPool,
    profile_id: ProfileId,
    finding: ResourceId,
) -> ApiResult<Vec<CitationAuditRow>> {
    let readable = readback::is_resource_visible(pool, profile_id, finding)
        .await
        .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?;
    if !readable {
        return Err(ApiError::NotFound(FINDING_REFUSAL.to_string()));
    }

    let rows = readback::citation_audit_trail(pool, profile_id, finding)
        .await
        .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?;

    Ok(rows
        .into_iter()
        .map(|r| CitationAuditRow {
            audit_id: r.audit_id,
            block_id: r.block_id,
            source_id: r.source_id,
            value: r.value,
            reason: r.reason,
            created: r.created,
            auditor_profile_id: r.auditor_profile_id,
            auditor_handle: r.auditor_handle,
            auditor_display_name: r.auditor_display_name,
        })
        .collect())
}
