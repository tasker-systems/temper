//! `DbBackend` struct + `impl Backend`. Per-request construction.

use sqlx::PgPool;

use temper_core::operations::Surface;
use temper_core::types::ids::ProfileId;

/// Postgres-backed backend impl. Constructed per inbound request.
///
/// Carries the request-scoped auth context (`profile_id`, `device_id`) and the
/// originating `Surface` so each command can be threaded into the existing
/// service-layer functions and so emitted events can be tagged appropriately.
// Fields and accessors are unused until Tasks 6-11 fill in the trait methods;
// suppress until the Backend impl lands.
#[allow(dead_code)]
pub struct DbBackend {
    pool: PgPool,
    profile_id: ProfileId,
    device_id: String,
    /// Origin of the inbound command. Stored for forward-compat (Phase 6
    /// telemetry/event tagging); not used by Phase 3a's coarse events.
    surface: Surface,
}

#[allow(dead_code)]
impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId, device_id: String, surface: Surface) -> Self {
        Self {
            pool,
            profile_id,
            device_id,
            surface,
        }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(crate) fn profile_id(&self) -> ProfileId {
        self.profile_id
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }
}
