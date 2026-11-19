//! Composite builders: an actor the way the integration suites seed one, built from the per-entity
//! builders. Use one when a test wants "a profile that can act through the doors and owns a
//! context"; use the per-entity builders when it wants anything narrower.

use sqlx::PgPool;
use uuid::Uuid;

use super::{context, entity, profile};

/// A door caller: a profile ([`profile::insert_with_email`]), one emitter per surface
/// ([`entity::insert_surfaces`]), and a personal context with slug and name `slug`.
#[derive(Debug, Clone)]
pub struct Caller {
    pub profile: Uuid,
    pub handle: String,
    /// The emitter ids, in the order of the surfaces asked for.
    pub emitters: Vec<Uuid>,
    pub context: Uuid,
}

/// [`Caller`] for `email`, acting through `surfaces`, owning a personal context `slug`.
pub async fn with_context(pool: &PgPool, email: &str, surfaces: &[&str], slug: &str) -> Caller {
    let (id, handle) = profile::insert_with_email(pool, email).await;
    let emitters = entity::insert_surfaces(pool, id, &handle, surfaces).await;
    let context = context::insert_personal(pool, id, slug, slug).await;
    Caller {
        profile: id,
        handle,
        emitters,
        context,
    }
}

/// The incumbent integration-suite caller: [`entity::DOOR_SURFACES`] and a context named `temper`.
pub async fn door_caller(pool: &PgPool, email: &str) -> Caller {
    with_context(pool, email, &entity::DOOR_SURFACES, "temper").await
}
