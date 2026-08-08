//! The v0 query-envelope contract. These types ARE the contract: the published spec is generated
//! from them in T3 and ships with the other generated artifacts, so there is no hand-written
//! second copy. See `docs/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md` for
//! the design reasoning.

pub mod act;
pub mod composition;
pub mod disposition;
pub mod envelope;
pub mod filter;
pub mod id_set;
pub mod registry;
pub mod scalars;
pub mod stage;
pub mod trace;
pub mod validate;

// `EdgeKind` is deliberately NOT re-exported here. It is not query's type — `crate::types` already
// re-exports it from `graph` — and a second public path to one type invites exactly the ambiguity
// that re-using it instead of restating it was meant to remove.
pub use act::{
    ActDeclaration, ActName, ActQuantity, BuildState, Disclosure, Door, DoorReach, QuantityScale,
    VisibilityProfile,
};
pub use composition::{
    CombineNode, CombineOp, Composition, Intention, OutcomeDeclaration, ReturnSpec, StageNode,
};
pub use disposition::{ActRefusal, RefusalReason, StageDisposition};
pub use envelope::{ActInvocation, NarrowedBy, StageResult};
pub use filter::{
    EdgeFilter, FacetPredicate, FilterField, PropertyOp, PropertyPredicate, PropertySubject,
    ResourceFilter,
};
pub use id_set::{IdKind, IdProvenance, IdSet};
pub use registry::{declaration, search_family};
pub use scalars::{BoundTerm, Extent, MetaDetail};
pub use stage::{StageInput, StageName, StageOutput, StageRelation};
pub use trace::{CompositionTrace, InputSource, MetaTruncated, StageTrace};
pub use validate::{emitted_fragment_for, validate, PlanRefusal, ValidatedComposition};
