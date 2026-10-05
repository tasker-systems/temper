//! Resource API types — shared between temper-api and temper-client.
//!
//! Every type here moved to [`temper_core::types::resource`]: they are wire types, and every wire
//! type is temper-core's (so a client names them without the server crates). Re-exported so each
//! `temper_workflow::types::resource::…` call site resolves unchanged.

pub use temper_core::types::resource::{
    BodyStorage, ContentChunk, ContentResponse, DeleteResponse, IngestState,
    ResourceAnnotateRequest, ResourceCreateRequest, ResourceFacets, ResourceListParams,
    ResourceListResponse, ResourceSortField, ResourceUpdateRequest, SortOrder,
};
