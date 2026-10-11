//! No code path can hold a `SystemAdmin` for a machine: `require_system_admin` takes a
//! `&HumanPrincipal`, and there is no conversion from `MachinePrincipal`. Asking the admin question
//! of a machine is a type error, not a refusal.
use temper_services::auth::{require_system_admin, MachinePrincipal};

async fn nope(pool: &sqlx::PgPool, machine: &MachinePrincipal) {
    // E0308: expected `&HumanPrincipal`, found `&MachinePrincipal`.
    let _ = require_system_admin(pool, machine).await;
}

fn main() {}
