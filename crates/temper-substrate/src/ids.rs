//! Typed-UUID newtypes — re-exported from the single canonical home, `temper_core::types::ids`.
//!
//! These were originally defined here (the convergence-guidance foundation) while substrate was
//! built side-by-side. They now live in temper-core so a mis-passed id is a compile error *across
//! crate boundaries* — there is one type per DB-row kind, shared by every crate. This module is a
//! thin re-export kept so substrate's `crate::ids::X` call sites stay stable.
//!
//! The newtypes encode/decode as a plain `Uuid` (transparent at the sqlx boundary). For
//! `sqlx::query!`/`query_as!`/`query_scalar!` macro binds, pass `id.0` or `id.uuid()`; runtime
//! `.bind(id)` and `row.get::<NewType, _>()` accept the newtype directly.

pub use temper_core::types::ids::{
    BlockId, ChunkId, CogmapId, ContextId, EdgeId, EntityId, EventId, InvocationId, LensId,
    ProfileId, PropertyId, RegionId, ResourceId,
};
