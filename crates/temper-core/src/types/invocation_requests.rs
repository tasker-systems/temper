//! Wire types for the `/api/invocations` endpoints. Shared between
//! `temper-api` (OpenAPI source) and `temper-client` (typed request builder).
//!
//! Cogmap/entity ids are substrate UUIDs, not resource refs: cogmaps and entities
//! are not resource-addressable. They come from the agent's launch / delegation
//! context, not `parse_ref`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::invocation::Disposition;

/// Request body for `POST /api/invocations`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct OpenInvocationRequest {
    /// Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`).
    pub trigger_kind: String,
    /// The cogmap the invocation operates on (substrate cogmap id).
    pub originating_cogmap: Uuid,
    /// Optional delegating-parent cogmap (must share a team with the originating
    /// cogmap — enforced by the substrate delegation gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_cogmap: Option<Uuid>,
}

/// Request body for `POST /api/invocations/{id}/close`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct CloseInvocationRequest {
    pub disposition: Disposition,
    /// Opaque terminal outcome payload (agent-defined shape).
    #[serde(default)]
    pub outcome: serde_json::Value,
}

/// Acknowledgement returned by the open endpoint — carries the minted
/// invocation id, fed back into the close call.
///
/// `id` is the generic create-response key (a duplicate of `invocation_id`), so a
/// caller's "create X, capture its id" helper reads `id` from every create-style ack
/// without knowing the per-command alias. Both surfaces (CLI + MCP) inherit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct InvocationAck {
    pub id: Uuid,
    pub invocation_id: Uuid,
}

/// Typed acknowledgement the MCP/CLI surfaces render after a close (the HTTP
/// endpoint itself returns 204 No Content). Echoes the closed invocation + its
/// terminal disposition — a structured row rather than inline JSON, shared so
/// both surfaces emit the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct InvocationCloseAck {
    pub invocation_id: Uuid,
    pub disposition: Disposition,
}

#[cfg(test)]
mod ack_tests {
    use super::*;

    #[test]
    fn invocation_ack_carries_both_id_and_invocation_id() {
        let iid = uuid::Uuid::nil();
        let ack = InvocationAck {
            id: iid,
            invocation_id: iid,
        };
        let v = serde_json::to_value(&ack).expect("serialize");
        assert_eq!(v["id"], v["invocation_id"], "id must alias invocation_id");
        assert!(v.get("id").is_some(), "generic `id` key present: {v}");
    }
}
