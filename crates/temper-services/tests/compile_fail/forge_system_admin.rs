//! `SystemAdmin` has a private field — constructing one by struct literal outside
//! `temper_services::auth` must not compile. This is the seal: minting a proof and running the gate
//! are the same act, so a forged proof cannot exist.
use temper_core::types::ids::ProfileId;
use temper_services::auth::SystemAdmin;

fn main() {
    // E0603: tuple struct constructor `SystemAdmin` is private.
    let _forged = SystemAdmin(ProfileId::from(uuid::Uuid::nil()));
}
