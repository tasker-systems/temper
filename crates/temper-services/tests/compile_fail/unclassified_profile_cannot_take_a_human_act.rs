//! An unclassified `AuthenticatedProfile` cannot stand in for a `&HumanPrincipal` either: the act
//! requires the classification to have run and said "person".
use temper_core::types::context::ShareContextRequest;
use temper_services::auth::AuthenticatedProfile;
use temper_services::services::context_service;

async fn nope(pool: &sqlx::PgPool, authed: &AuthenticatedProfile, req: &ShareContextRequest) {
    // E0308: expected `&HumanPrincipal`, found `&AuthenticatedProfile`.
    let _ = context_service::share(pool, authed, uuid::Uuid::nil(), req).await;
}

fn main() {}
