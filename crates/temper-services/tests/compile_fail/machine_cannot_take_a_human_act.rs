//! An act a machine may not take is typed to require a `&HumanPrincipal`. A machine's proof cannot
//! stand in for one — sharing a context is the example; every refused act has the same shape.
use temper_core::types::context::ShareContextRequest;
use temper_services::auth::MachinePrincipal;
use temper_services::services::context_service;

async fn nope(pool: &sqlx::PgPool, machine: &MachinePrincipal, req: &ShareContextRequest) {
    // E0308: expected `&HumanPrincipal`, found `&MachinePrincipal`.
    let _ = context_service::share(pool, machine, uuid::Uuid::nil(), req).await;
}

fn main() {}
