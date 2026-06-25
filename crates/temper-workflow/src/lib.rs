//! temper-workflow — the domain-A (knowledge-base / workflow) type and logic
//! cluster extracted from temper-core.
//!
//! This crate owns the doctype-aware vocabulary: resource rows, managed
//! metadata, the frontmatter model (incl. `DocType`), schema validation,
//! the vault projection, doc-type defaults, the operations command layer
//! (`Backend` trait), the managed-metadata hash, and the `DocType`-dependent
//! half of the knowledge graph. It depends only on the neutral `temper-core`
//! leaf.

pub mod defaults;
pub mod frontmatter;
pub mod hash;
pub mod operations;
pub mod schema;
pub mod types;
pub mod vault;
