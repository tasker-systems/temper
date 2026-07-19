//! `temper trail` business logic — thin wrapper over the events client's
//! element-trail read. Cloud-only; the trail is served, gated, from the API.

use temper_core::types::element_trail::{ElementKind, EventTrail};
use uuid::Uuid;

use crate::error::Result;

/// Call the element-trail API for a graph element (node or edge), already
/// resolved to a UUID.
pub async fn element_trail_api(
    client: &temper_client::TemperClient,
    kind: ElementKind,
    element_id: Uuid,
) -> Result<EventTrail> {
    client
        .events()
        .element_trail(kind, element_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}
