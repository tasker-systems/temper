//! `temper steward` business logic — thin wrappers over the steward sub-client. Cloud-only.

use uuid::Uuid;

use temper_core::types::steward::{AdvanceWatermarkAck, IngestDelta};

use crate::error::Result;

/// Read the ingest delta for a cogmap since its watermark.
pub async fn delta_api(
    client: &temper_client::TemperClient,
    cogmap: Uuid,
    threshold: Option<i64>,
) -> Result<IngestDelta> {
    client
        .steward()
        .delta(cogmap, threshold)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}

/// Advance the cogmap's ingest watermark to a given event id.
pub async fn advance_watermark_api(
    client: &temper_client::TemperClient,
    cogmap: Uuid,
    event_id: Uuid,
) -> Result<AdvanceWatermarkAck> {
    client
        .steward()
        .advance_watermark(cogmap, event_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}
