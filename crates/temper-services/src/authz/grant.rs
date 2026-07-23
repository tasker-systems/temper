//! Grant administration — may this caller administer grants on this subject?
//!
//! The arms and the probe sequence are unchanged from the `grant_authority` this replaces; only
//! the shape moved. In particular the **short-circuit order is load-bearing**: an admin costs one
//! query, a denied delegate costs three, and reordering the branches would quietly make every
//! admin request pay for probes it does not need.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_substrate::payloads::{AnchorTable, RefTarget};

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::access_service::{
    cogmap_write_requires_admin, is_system_admin, profile_can_grant, GrantAuthority,
};

#[async_trait]
impl ScopedAuthority for GrantAuthority {
    type Subject = RefTarget;

    async fn resolve(pool: &PgPool, caller: ProfileId, subject: RefTarget) -> ApiResult<Self> {
        if is_system_admin(pool, caller).await? {
            return Ok(GrantAuthority::SystemAdmin);
        }

        // Structural escalation guard (plan Task 5b.4). `require_cogmap_write_admin` keeps the
        // reserved L0 kernel and gating-team-joined maps admin-only, but the grant path never
        // consulted it — so a non-admin `can_grant` holder could mint `can_write` on the kernel,
        // reaching by the grant axis exactly what the write axis forbids. `machine_authz`'s own
        // tests seed such a row, so the state is reachable, not hypothetical. Admins already
        // returned above, so a map in the admin-only regime denies here regardless of any
        // `can_grant` the caller holds.
        if subject.kind == AnchorTable::Cogmaps
            && cogmap_write_requires_admin(pool, CogmapId(subject.id)).await?
        {
            return Ok(GrantAuthority::None);
        }

        Ok(
            if profile_can_grant(pool, caller, subject.kind.as_str(), subject.id).await? {
                GrantAuthority::Delegated
            } else {
                GrantAuthority::None
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, GrantAuthority::None)
    }

    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}

/// Type a wire-supplied `subject_table` string, denying rather than erroring on an unknown value.
///
/// **`None` means denied, not malformed.** That is deliberate parity: before this existed, an
/// unrecognized table string was passed straight to the `can(...)` SQL predicate, which matched no
/// row and returned false — a 403. Raising a 400 here instead would be a behavior change on a path
/// that has no live caller to change it for.
///
/// It has no live caller because every surface *injects* the table as a literal
/// (`handlers/resources.rs:430` and `:466`, `handlers/cognitive_maps.rs:365` and `:400`, and the
/// MCP equivalents) and a DB CHECK bounds the column to those same four values
/// (`migrations/20260714000020_connection_reach_grants.sql:44`). The `String` on the wire type is a
/// stringly-typed spelling of a set the callers already know statically; this function is the one
/// place that has to cope with that, and it copes by denying.
pub(crate) fn wire_subject(subject_table: &str, subject_id: Uuid) -> Option<RefTarget> {
    // No `_ =>` over the admitted set: adding a table to the CHECK constraint should surface here
    // as a decision, not resolve silently to a denial.
    let kind = match subject_table {
        "kb_resources" => AnchorTable::Resources,
        "kb_contexts" => AnchorTable::Contexts,
        "kb_cogmaps" => AnchorTable::Cogmaps,
        "kb_connections" => AnchorTable::Connections,
        _ => return None,
    };
    Some(RefTarget {
        kind,
        id: subject_id,
    })
}
