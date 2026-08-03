//! The v0 query-envelope contract. These types ARE the contract: the published spec is generated
//! from them in T3 and ships with the other generated artifacts, so there is no hand-written
//! second copy. See `docs/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md` for
//! the design reasoning.

pub mod filter;
pub mod id_set;
pub mod scalars;

// `EdgeKind` is deliberately NOT re-exported here. It is not query's type — `crate::types` already
// re-exports it from `graph` — and a second public path to one type invites exactly the ambiguity
// that re-using it instead of restating it was meant to remove.
pub use filter::{EdgeFilter, FacetPredicate, FilterField, ResourceFilter};
pub use id_set::{IdKind, IdProvenance, IdSet};
pub use scalars::{BoundTerm, BoundsMode, Extent, MetaDetail};
