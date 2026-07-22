//! A pure-admin service fn requires `&SystemAdmin` — calling it without one must not compile. This is
//! the enclosure: an ungated call path to a privileged act is a compile error, not a forgotten check.
//! A `ProfileId` (or any other value) cannot stand in for the proof.
use temper_core::types::ids::ProfileId;
use temper_services::services::access_service;

async fn nope(pool: &sqlx::PgPool, subject: ProfileId) {
    // `admin_revoke` is `(pool, &SystemAdmin, ProfileId, String)`. Correct arity, but no proof in hand:
    // a `ProfileId` is not a `&SystemAdmin`, so this is a type error at argument #2.
    let _ = access_service::admin_revoke(pool, subject, subject, String::new()).await;
}

fn main() {}
