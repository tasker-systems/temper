//! Authoritative frontmatter handling for temper vault files.
//!
//! This module is the single source of truth for parsing, validating,
//! mutating, tier-splitting, hashing, and writing YAML frontmatter in
//! vault markdown files. The central type is `Frontmatter`, an
//! aggregate that owns the canonicalized YAML and exposes typed
//! projections via standard trait impls.
//!
//! Hash computation delegates unchanged to `crate::hash::compute_managed_hash`
//! and `crate::hash::compute_open_hash` from PR #40 — `Frontmatter::hashes()`
//! never introduces a new canonicalization algorithm for hashing. The
//! display-ordering algorithm in [`canonical`] is strictly for on-disk
//! writes and has zero effect on hash output.

pub mod canonical;
pub mod document;
pub mod fields;
pub mod parse;
pub mod projections;
pub mod registry;
pub mod tiers;

// Session 1 Task 1 stub — Tasks 3/7 uncomment:
// pub use document::{DocType, Frontmatter};
// pub use registry::{
//     FieldCategory, KnownOpenField, OpenFieldType, KNOWN_OPEN_FIELDS,
// };
