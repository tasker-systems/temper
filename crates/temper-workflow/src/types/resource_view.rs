//! `ResourceView` — re-exported from its canonical home, [`temper_core::types::resource_view`].
//!
//! The type moved to temper-core because temper-substrate's `hit_identities` answers in it
//! directly, and substrate is the layer *below* this crate. This module keeps every
//! `temper_workflow::types::resource_view::…` call site resolving unchanged.

pub use temper_core::types::resource_view::{ResourceSection, ResourceView, SectionSet};
