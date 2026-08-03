//! The v0 query-envelope contract. These types ARE the contract: the published spec is generated
//! from them in T3 and ships with the other generated artifacts, so there is no hand-written
//! second copy. See `docs/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md` for
//! the design reasoning.

pub mod id_set;
pub mod scalars;

pub use id_set::{IdKind, IdProvenance, IdSet};
pub use scalars::{BoundTerm, BoundsMode, Extent, MetaDetail};
