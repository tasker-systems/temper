//! Resource wire types.
//!
//! The legacy `public`-shape read/write fns retired with the flip — reads route
//! through the `substrate_read` dispatcher (`readback`), writes through
//! `DbBackend`. What remains is the re-export of the resource wire types the
//! handlers + `substrate_read` address by their `crate::services::resource_service`
//! path.

pub use temper_core::types::resource::{
    ContentChunk, ContentResponse, ResourceCreateRequest, ResourceFacets, ResourceListParams,
    ResourceListResponse, ResourceRow, ResourceSortField, ResourceUpdateRequest, SortOrder,
};
