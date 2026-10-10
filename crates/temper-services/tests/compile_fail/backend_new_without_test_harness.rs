//! `DbBackend::new` — a backend acting as a bare id, with no proof behind it — exists only under the
//! `test-harness` feature. A production build (this job compiles without it) cannot name it, so every
//! backend a deployed binary builds was constructed from a classified caller.
use temper_core::types::ids::ProfileId;
use temper_services::backend::DbBackend;

fn nope(pool: sqlx::PgPool) {
    // E0599: no function or associated item named `new` found for struct `DbBackend`.
    let _ = DbBackend::new(pool, ProfileId::from(uuid::Uuid::nil()));
}

fn main() {}
