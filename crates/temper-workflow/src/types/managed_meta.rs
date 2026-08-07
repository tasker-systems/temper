//! Managed (`temper-*`) frontmatter — re-exported from [`temper_core::types::managed_meta`].
//!
//! [`ManagedMeta`] and friends moved to temper-core because `ManagedMeta` is a field of
//! `ResourceView`, which temper-substrate — the layer *below* this crate — reads back
//! directly. Every `temper_workflow::types::managed_meta::…` call site resolves unchanged.
//!
//! Nothing in this family is defined here any more. `ResourceMetaListResponse` was, because its
//! rows were `ResourceDetail`s — a temper-workflow type. The list endpoint has one envelope and
//! one row type now ([`crate::types::resource::ResourceListResponse`] over `ResourceView`), so
//! the second envelope has no shape left to carry.

pub use temper_core::types::managed_meta::{
    ManagedMeta, MetaUpdatePayload, ResourceManifestRow, PROVENANCE_LLM_DISCOVERED,
    PROVENANCE_USER_CREATED,
};
