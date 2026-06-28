//! `temper invocation` business logic — thin wrappers over the invocations
//! sub-client. Cloud-only.

use uuid::Uuid;

use temper_core::types::invocation::{InvocationSummary, InvocationView};
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationAck, OpenInvocationRequest,
};

use crate::error::Result;

/// Open an invocation envelope. Returns the minted-id acknowledgement.
pub async fn open_api(
    client: &temper_client::TemperClient,
    req: &OpenInvocationRequest,
) -> Result<InvocationAck> {
    client
        .invocations()
        .open(req)
        .await
        .map_err(crate::commands::client_err)
}

/// Close an open invocation envelope (204 No Content on success).
pub async fn close_api(
    client: &temper_client::TemperClient,
    invocation_id: Uuid,
    req: &CloseInvocationRequest,
) -> Result<()> {
    client
        .invocations()
        .close(invocation_id, req)
        .await
        .map_err(crate::commands::client_err)
}

/// Read one envelope plus its acts.
pub async fn show_api(
    client: &temper_client::TemperClient,
    invocation_id: Uuid,
) -> Result<InvocationView> {
    client
        .invocations()
        .show(invocation_id)
        .await
        .map_err(crate::commands::client_err)
}

/// List envelopes, optionally narrowed by cogmap and/or status.
pub async fn list_api(
    client: &temper_client::TemperClient,
    cogmap: Option<Uuid>,
    status: Option<String>,
) -> Result<Vec<InvocationSummary>> {
    client
        .invocations()
        .list(cogmap, status)
        .await
        .map_err(crate::commands::client_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn render_invocation_view_json_is_passthrough_object() {
        let view = InvocationView {
            id: Uuid::from_u128(1),
            status: "open".to_string(),
            trigger_kind: "manual".to_string(),
            originating_cogmap_id: Uuid::from_u128(2),
            parent_cogmap_id: None,
            scoped_entity_id: Uuid::from_u128(3),
            telos_resource_id: Uuid::from_u128(4),
            outcome: None,
            opened_at: Utc::now(),
            closed_at: None,
            acts: vec![],
        };
        let out =
            crate::format::render(&view, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.contains("\"trigger_kind\""), "json: {out}");
        assert!(out.contains("\"status\""), "json: {out}");
    }

    #[test]
    fn render_invocation_summaries_json_is_passthrough_array() {
        let rows: Vec<InvocationSummary> = vec![InvocationSummary {
            id: Uuid::from_u128(1),
            status: "completed".to_string(),
            trigger_kind: "manual".to_string(),
            originating_cogmap_id: Uuid::from_u128(2),
            opened_at: Utc::now(),
            closed_at: Some(Utc::now()),
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"originating_cogmap_id\""), "json: {out}");
    }
}
