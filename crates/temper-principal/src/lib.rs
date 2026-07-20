//! The principal-admission machines (spec 2026-07-20 §4).
//!
//! Two machines, deliberately separated (D1): a **persisted** `Standing` lifecycle whose
//! transitions this crate validates, and a **pure, per-request** `Admission` decision that reads
//! standing as evidence.
//!
//! # Why this is its own crate
//!
//! Every `match` over [`Standing`] here is exhaustive with no `_ =>` arm, so adding a state
//! becomes a compile error at every decision site (spec §7 obligation 3). That property is what
//! the crate boundary buys; it cannot be bought by discipline inside a larger crate.
//!
//! This crate performs no I/O, holds no identifiers, and never resolves a credential. It judges
//! assembled evidence — which is what makes it safe to share across surfaces (spec §4).

mod act;
mod admission;
mod refusal;
mod standing;
mod transition;

// Re-exports are restored by the tasks that fill each module:
//   Task 2 — act.rs, transition.rs
//   Task 3 — admission.rs, refusal.rs
// pub use act::{Act, ActorAuthority, Provisioner};
// pub use admission::{admit, AdmittedPrincipal};
// pub use refusal::Refusal;
pub use standing::Standing;
// pub use transition::transition;
