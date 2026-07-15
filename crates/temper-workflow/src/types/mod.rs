//! Domain-A (workflow / knowledge-base) types — resource rows, managed
//! metadata, and the `DocType`-dependent half of the knowledge graph.
//!
//! The neutral structural edge taxonomy (`EdgeKind`/`Polarity`) lives in
//! `temper_core::types::graph`.

pub mod graph;
pub mod managed_meta;
pub mod resource;

pub use graph::{
    EdgeReconciliation, EdgeType, GraphEdgeRow, GraphNeighborRow, GraphTraversalRow, ResolvedEdge,
    ResourceRelationships, TargetRef,
};
pub use managed_meta::{ManagedMeta, MetaUpdatePayload, ResourceManifestRow};
pub use resource::{
    BodyStorage, ContentChunk, ContentResponse, DeleteResponse, IngestState, ResourceCreateRequest,
    ResourceFacets, ResourceListParams, ResourceListResponse, ResourceRow, ResourceSortField,
    ResourceUpdateRequest, SortOrder,
};
