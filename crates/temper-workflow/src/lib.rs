//! temper-workflow — the domain-A (knowledge-base / workflow) type and logic
//! cluster extracted from temper-core.
//!
//! This crate owns the doctype-aware vocabulary: resource rows, managed
//! metadata, the frontmatter model (incl. `DocType`), schema validation,
//! the vault projection, doc-type defaults, the operations frame (managed-meta actions,
//! `parse_ref`, the act surface), the per-act request shaping every door uses, the
//! managed-metadata hash, and the `DocType`-dependent half of the knowledge graph. It depends
//! only on the neutral `temper-core` leaf.
//!
//! The server's write-command vocabulary (`CreateResource` and its siblings, `CommandOutput`,
//! `DomainEvent`) is not published here: it lives in `temper-services`, the one crate that runs
//! it.

pub mod defaults;
pub mod frontmatter;
pub mod hash;
pub mod operations;
pub mod schema;
pub mod shaping;
pub mod types;
pub mod vault;
